use log::error;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    net::UnixStream,
};

use crate::error::Error;

pub fn socket_path() -> Result<String, Error> {
    std::env::var(niri_ipc::socket::SOCKET_PATH_ENV)
        .map_err(|_| Error::Local("NIRI_SOCKET not set".to_string()))
}

pub async fn niri_request(request: niri_ipc::Request) -> Result<niri_ipc::Response, Error> {
    let path = socket_path()?;
    let mut stream = UnixStream::connect(&path).await
        .map_err(|e| Error::Local(format!("Failed to connect to niri socket: {e}")))?;

    let mut json = serde_json::to_string(&request)
        .map_err(|e| Error::Local(format!("Failed to serialize request: {e}")))?;
    json.push('\n');
    stream.write_all(json.as_bytes()).await
        .map_err(|e| Error::Local(format!("Failed to write to niri socket: {e}")))?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await
        .map_err(|e| Error::Local(format!("Failed to read from niri socket: {e}")))?;

    let reply: niri_ipc::Reply = serde_json::from_str(&line)
        .map_err(|e| Error::Local(format!("Failed to parse niri reply: {e}")))?;

    reply.map_err(|e| Error::Local(format!("niri error: {e}")))
}

pub type EventLines = Lines<BufReader<tokio::io::ReadHalf<UnixStream>>>;

pub async fn open_event_stream() -> Result<EventLines, Error> {
    let path = socket_path()?;
    let stream = UnixStream::connect(&path).await
        .map_err(|e| Error::Local(format!("Failed to connect to niri socket: {e}")))?;

    let (reader, mut writer) = tokio::io::split(stream);
    let request = serde_json::to_string(&niri_ipc::Request::EventStream).unwrap() + "\n";
    writer.write_all(request.as_bytes()).await
        .map_err(|e| Error::Local(format!("Failed to send EventStream request: {e}")))?;

    let mut lines = BufReader::new(reader).lines();

    // First line is the Reply to our EventStream request
    match lines.next_line().await {
        Ok(Some(line)) => {
            if let Ok(reply) = serde_json::from_str::<niri_ipc::Reply>(&line) {
                if let Err(err) = reply {
                    return Err(Error::Local(format!("niri EventStream error: {err}")));
                }
            }
        }
        Ok(None) => return Err(Error::Local("niri event stream closed immediately".to_string())),
        Err(err) => return Err(Error::Local(format!("Error reading EventStream reply: {err}"))),
    }

    Ok(lines)
}

pub async fn next_event(lines: &mut EventLines) -> Option<niri_ipc::Event> {
    match lines.next_line().await {
        Ok(Some(line)) => serde_json::from_str(&line).ok(),
        Ok(None) => None,
        Err(err) => {
            error!("Error reading niri event stream: {err}");
            None
        }
    }
}
