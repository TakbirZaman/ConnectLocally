//! Headless validation runner. Commands are newline-delimited JSON on stdin.
use anyhow::{Context, Result};
use localconnect_core::engine::{Config, Engine, Peer};
use tokio::io::{AsyncBufReadExt, BufReader};
#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    anyhow::ensure!(
        args.len() >= 4,
        "usage: localconnect DATA_DIR RECEIVE_DIR NAME [PORT]"
    );
    let e = Engine::start(Config {
        data_dir: args[1].clone(),
        receive_dir: args[2].clone(),
        name: args[3].clone(),
        port: args.get(4).map(|s| s.parse()).transpose()?.unwrap_or(53317),
        in_flight: 1,
    })
    .await?;
    println!(
        "{}",
        serde_json::json!({"localFingerprint":e.identity.fingerprint,"port":e.port})
    );
    let mut events = e.events.subscribe();
    let stop = e.cancellation();
    let printer = tokio::spawn(async move {
        loop {
            tokio::select! {_=stop.cancelled()=>break,event=events.recv()=>match event {Ok(event)=>println!("{}",serde_json::to_string(&event).unwrap()),Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>continue,Err(_)=>break}}
        }
    });
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let result = command(&e, &line).await;
        match result {
            Ok(value) => println!("{}", serde_json::json!({"result":value})),
            Err(error) => println!("{}", serde_json::json!({"error":error.to_string()})),
        }
        if serde_json::from_str::<serde_json::Value>(&line)
            .ok()
            .is_some_and(|v| v["command"] == "stop")
        {
            break;
        }
    }
    e.shutdown().await;
    let _ = printer.await;
    Ok(())
}
async fn command(e: &std::sync::Arc<Engine>, line: &str) -> Result<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_str(line)?;
    let field = |key: &str| -> Result<String> {
        Ok(v[key]
            .as_str()
            .context(format!("missing {key}"))?
            .to_owned())
    };
    Ok(match field("command")?.as_str() {
        "probe" => serde_json::to_value(e.probe(field("address")?).await?)?,
        "pair" => serde_json::to_value(e.pair(field("address")?, field("fingerprint")?).await?)?,
        "approve" => {
            e.approve(field("fingerprint")?, v["allow"].as_bool().unwrap_or(false))
                .await?;
            serde_json::json!("approved decision delivered")
        }
        "connect" => {
            e.start_control(serde_json::from_value::<Peer>(v["peer"].clone())?)
                .await?;
            serde_json::json!("connecting")
        }
        "send" => serde_json::json!(
            e.send(
                field("path")?,
                serde_json::from_value::<Peer>(v["peer"].clone())?
            )
            .await?
        ),
        "resume" => {
            e.resume(field("id")?).await?;
            serde_json::json!("resuming")
        }
        "cancel" => {
            e.cancel(field("id")?).await?;
            serde_json::json!("cancelled")
        }
        "forget" => {
            e.forget(field("fingerprint")?).await?;
            serde_json::json!("forgotten")
        }
        "history" => serde_json::from_str(&e.history().await?)?,
        "peers" => serde_json::from_str(&e.peers().await?)?,
        "stop" => serde_json::json!("stopping"),
        _ => anyhow::bail!("unknown command"),
    })
}
