use crate::errors::{HassError, HassResult};
use crate::messages::Response;
use crate::runtime::{connect_async, task, WebSocket};

use async_tungstenite::tungstenite::protocol::Message;
use futures::channel::mpsc::{channel, Receiver, Sender};
use futures::lock::Mutex;
use futures::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use std::collections::HashMap;
use std::sync::Arc;
use url;
use uuid::Uuid;

#[derive(Debug)]
pub enum Cmd {
    Msg((Sender<HassResult<Response>>, Uuid, Vec<u8>)),
    Pong(Vec<u8>),
    Shutdown,
}
#[derive(Debug)]
pub struct WsConn {
    pub(crate) sender: Sender<Cmd>,
}

impl WsConn {
    pub async fn connect(url: url::Url) -> HassResult<WsConn> {       
        let client = connect_async(url).await?;
        let (sink, stream) = client.split();
        let (sender, receiver) = channel::<Cmd>(20);
        let requests: Arc<Mutex<HashMap<Uuid, Sender<HassResult<Response>>>>> = Arc::new(Mutex::new(HashMap::new()));
        
        sender_loop(sink, requests.clone(), receiver);
        receiver_loop(stream, requests.clone(), sender.clone());

        Ok(WsConn { sender })
    }

    pub async fn run(
        &mut self,
        id: Uuid,
        payload: Vec<u8>,
    ) -> HassResult<(Response, Receiver<HassResult<Response>>)> {
        let (sender, mut receiver) = channel(1);

        self.sender.send(Cmd::Msg((sender, id, payload))).await?;

        receiver
            .next()
            .await
            .expect("It should contain the response")
            .map(|r| (r, receiver))
    }
}

fn sender_loop(
    mut sink: SplitSink<WebSocket, Message>,
    requests: Arc<Mutex<HashMap<Uuid, Sender<HassResult<Response>>>>>,
    mut receiver: Receiver<Cmd>,
) {
    task::spawn(async move {
        loop {
            match receiver.next().await {
                Some(item) => match item {
                    Cmd::Msg(msg) => {
                        let mut guard = requests.lock().await;
                        guard.insert(msg.1, msg.0);
                        if let Err(e) = sink.send(Message::Binary(msg.2)).await {
                            let mut sender = guard.remove(&msg.1).unwrap();
                            sender
                                .send(Err(HassError::from(e)))
                                .await
                                .expect("Failed to send error");
                        }
                        drop(guard);
                    }
                    Cmd::Pong(data) => {
                        sink.send(Message::Pong(data))
                            .await
                            .expect("Failed to send pong message.");
                    }
                    Cmd::Shutdown => {
                        let mut guard = requests.lock().await;
                        guard.clear();
                    }
                },
                None => {}
            }
        }
    });
}

fn receiver_loop(
    mut stream: SplitStream<WebSocket>,
    requests: Arc<Mutex<HashMap<Uuid, Sender<HassResult<Response>>>>>,
    mut sender: Sender<Cmd>,
) {
    task::spawn(async move {
        loop {
            match stream.next().await {
                Some(Err(error)) => {
                    let mut guard = requests.lock().await;
                    for s in guard.values_mut() {
                        match s.send(Err(HassError::from(&error))).await {
                            Ok(_r) => {}
                            Err(_e) => {}
                        }
                    }
                    guard.clear();
                }
                Some(Ok(item)) => match item {
                    Message::Binary(data) => {
                        let response: Response = serde_json::from_slice(&data).unwrap();
                        let mut guard = requests.lock().await;
                        if response.status.code != 206 {
                            let item = guard.remove(&response.request_id);
                            drop(guard);
                            if let Some(mut s) = item {
                                match s.send(Ok(response)).await {
                                    Ok(_r) => {}
                                    Err(_e) => {}
                                };
                            }
                        } else {
                            let item = guard.get_mut(&response.request_id);
                            if let Some(s) = item {
                                match s.send(Ok(response)).await {
                                    Ok(_r) => {}
                                    Err(_e) => {}
                                };
                            }
                            drop(guard);
                        }
                    }
                    Message::Ping(data) => sender.send(Cmd::Pong(data)).await.unwrap(),
                    _ => {}
                },
                None => {}
            }
        }
    });
}