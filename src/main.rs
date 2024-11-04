use std::time::{Duration, Instant, UNIX_EPOCH};

use actix_sse::sse_channel;
use actix_web::{ get, HttpResponse, Responder};
use tokio::time::sleep;


pub mod actix_sse;


#[get("/sse")]
async fn sse_endpoint() -> impl Responder {
    let (mut sender, receiver) = sse_channel(10);

    tokio::spawn(async move {
        let mut i = 0;
        let start = Instant::now();
        sender.send_event("connect").await.expect("Socket closed");


        while i < 10 {
            sleep(Duration::from_secs(1)).await;
            let elapsed = start.elapsed().as_millis();
            sender.send_event_with_data("elapsed", elapsed).await.expect("Socket closed");
            i += 1;
        }
        
        sender.send_event("end").await.expect("Socket closed");

        sleep(Duration::from_secs(2)).await; // Wait for client to close the connection

        println!("Short sse completed")
    });

    receiver
}

#[get("/long-sse")]
async fn long_sse() -> impl Responder {
    let (mut sender, receiver) = sse_channel(10);

    tokio::spawn(async move {
        let start = Instant::now();

        loop {
            sleep(Duration::from_secs(5)).await;
            let elapsed = start.elapsed().as_millis();
            sender.send_event_with_data("elapsed", elapsed).await.expect("Socket closed");
        }
    });

    receiver
}



#[get("/")]
async fn index() -> impl Responder {
    let index = include_str!("./index.html");
    HttpResponse::Ok().content_type("text/html").body(index)
}


#[actix_web::main]
async fn main() {
    println!("Starting server on http://127.0.0.1:8080/");
    actix_web::HttpServer::new(|| actix_web::App::new().service(sse_endpoint).service(long_sse).service(index))
       .bind("127.0.0.1:8080")
       .unwrap()
       .run()
       .await
       .unwrap();
}
