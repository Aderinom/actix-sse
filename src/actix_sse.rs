use std::time::Duration;

use actix_web::{body::BoxBody, http::header::ContentEncoding, web::Bytes, HttpResponse};
use serde::Serialize;
use tokio::{sync::mpsc, time::{interval, Interval, MissedTickBehavior}};

/// Interval in which the SSE Sender will chcek if the client is still connected.
/// Actix does not do this without us trying to send data in the Stream.
const SSE_PING_INTERVAL: Duration = Duration::from_secs(5);

/// Keep alive comment to check for the client's presence.
const SSE_PING_COMMENT: Bytes = Bytes::from_static(b": keep-alive\n\n");

///
/// Creates a new Server-Sent Events (SSE) channel with a specified buffer size.
///
/// ```ignore
/// #[get("/sse")]
/// async fn sse_endpoint() -> impl Responder {
///     let (mut sender, receiver) = sse_channel(10);
/// 
///     tokio::spawn(async move {
///         let mut i = 0;
///         sender.send_event("hello").await.unwrap();
///         while i < 10 {
///             sleep(Duration::from_secs(1)).await;
///             sender.send_event("event").await.unwrap();
///             i += 1;
///         }
///         sender.send_event("bye").await.unwrap();
///     });
/// 
///     return receiver;
/// }
/// ```
///
pub fn sse_channel(buffer: usize) -> (SSESender, SSEReceiver) {
    let (sender, receiver) = mpsc::channel(buffer);
    let sender = SSESender::new(sender);
    let receiver = SSEReceiver::new(receiver);
    return (sender, receiver);
}

/// Server Side Event (SSE) Sending channel
///
/// Allows sending SSE events over a connection.
///
/// When the sender is dropped, the SSE stream will be closed after flushing all messages.
///
#[derive(Clone)]
pub struct SSESender {
    sender: mpsc::Sender<Bytes>,
}

#[derive(thiserror::Error, Debug)]
pub enum SSESendError {
    #[error("Failed to serialize data: {0}")]
    SerializeFailed(#[from] serde_json::Error),
    #[error("SSE connection was closed")]
    ConnectionClosed,
}

impl SSESender {
    fn new(sender: mpsc::Sender<Bytes>) -> SSESender {
        SSESender { sender }
    }


    /// Completes when the sender was closed
    /// Either the stream has completed or the connection was closed.
    pub async fn on_close(&self) {
        self.sender.closed().await;
    }

    pub fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }

    /// Sends a simple event with no data.
    /// Format `event: {event}\ndata:\n\n`
    pub async fn send_event(&mut self, event: &str) -> Result<(), SSESendError> {
        let bytes = Bytes::from(format!("event: {event}\ndata:\n\n"));
        self.send_bytes(bytes).await
    }

    /// Sends an event with data.
    /// Format `event: {event}\ndata: {data}\n\n``
    pub async fn send_event_with_data(
        &mut self,
        event: &str,
        data: impl Serialize,
    ) -> Result<(), SSESendError> {
        let serialized_data = serde_json::to_string(&data)?;
        let bytes = Bytes::from(format!("event: {event}\ndata: {serialized_data}\n\n"));
        self.send_bytes(bytes).await
    }

    /// Just sends data without an event.
    /// Format `data: {data}\n\n`
    pub async fn send_data(&mut self, data: impl Serialize) -> Result<(), SSESendError> {
        let serialized_data = serde_json::to_string(&data)?;
        let bytes = Bytes::from(format!("data: {serialized_data}\n\n"));
        self.send_bytes(bytes).await
    }

    /// Helper to just send the bytes to the sender and map to our internal Error
    async fn send_bytes(&mut self, bytes: Bytes) -> Result<(), SSESendError> {
        self.sender
            .send(bytes)
            .await
            .map_err(|e| SSESendError::ConnectionClosed)
    }

    async fn try_send_bytes(&mut self, bytes: Bytes) -> Result<(), SSESendError> {
        self.sender
            .try_send(bytes)
            .map_err(|e| SSESendError::ConnectionClosed)
    }
}

/// Server Side Event (SSE) Receiving channel - to be used as a response in an actix endpoint
///  
/// Receiver of SSE events, can just be passed as a response in any SSE endpoint  
/// Will automatically be dropped when the connection is closed or the sender is dropped  
pub struct SSEReceiver {
    receiver: mpsc::Receiver<Bytes>,
    ping_interval: Interval,
}

impl SSEReceiver {
    fn new(receiver: mpsc::Receiver<Bytes>) -> Self {
        let mut interval = interval(SSE_PING_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

        SSEReceiver { receiver, ping_interval: interval  }
    }
}

// Allows the SSEReceiver to be used as a response in an actix endpoint
impl actix_web::Responder for SSEReceiver {
    type Body = BoxBody;

    fn respond_to(self, req: &actix_web::HttpRequest) -> HttpResponse<Self::Body> {
        HttpResponse::Ok()
            .insert_header(("content-type", "text/event-stream"))
            .insert_header(("cache-control", "no-cache"))
            .insert_header(ContentEncoding::Identity)
            .insert_header(("X-Accel-Buffering", "no"))
            .streaming(self)
            .respond_to(req)
    }
}

// Allows the SSEReceiver to be used as a streaming response
impl futures::Stream for SSEReceiver {
    type Item = Result<Bytes, actix_web::Error>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {

        let this = self.get_mut();

        // Poll underlying channel
        if let std::task::Poll::Ready(res) = this.receiver.poll_recv(cx) {
            return std::task::Poll::Ready(res.map(Ok));
        }

        // Send ping comment if interval has passed
        if this.ping_interval.poll_tick(cx).is_ready() {
            // We are passing the SSEReceiver as a "StreamBody"
            // (actix 4.5.1) It seems that actix will only check if the connection to the client is still open when the stream body returned a Ready with data.
            // Just waking the SSEReceiver's stream future on a regular interval also does not make Actix check if the connection is still open.
            return std::task::Poll::Ready(Some(Ok(SSE_PING_COMMENT.clone())));
        }

        
        std::task::Poll::Pending
    }
}
