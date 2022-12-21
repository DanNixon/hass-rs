use env_logger;
use hass_rs::{client, WSEvent};
use lazy_static::lazy_static;
use std::env::var;

lazy_static! {
    static ref TOKEN: String =
        var("HASS_TOKEN").expect("please set up the HASS_TOKEN env variable before running this");
}

#[cfg_attr(feature = "async-std-runtime", async_std::main)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    println!("Creating the Websocket Client and Authenticate the session");
    let mut client = client::connect("homeassistant.castle.dan-nixon.com").await?;

    client.auth_with_longlivedtoken(&*TOKEN).await?;
    println!("WebSocket connection and authethication works");

    println!("Subscribe to an Event");
    let pet = |item: WSEvent| {
        println!("doot: {:#?}", item);
    };

    let id = client.subscribe_event("state_changed", pet).await.unwrap();

    async_std::task::sleep(std::time::Duration::from_secs(20)).await;

    Ok(())
}
