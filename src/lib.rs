use std::{
    collections::HashSet,
    error::Error,
    time::{SystemTime, UNIX_EPOCH},
};

use hyper::{
    body::{Bytes, HttpBody},
    Client,
};
use hyperlocal::{UnixClientExt, UnixConnector};
use once_cell::sync::Lazy;
use tokio::{sync::mpsc::UnboundedReceiver, task::JoinHandle};

static UNIX_CLIENT: Lazy<Client<UnixConnector>> = Lazy::new(|| Client::unix());
static CONTAINERS_ENDPOINT: Lazy<hyper::Uri> =
    Lazy::new(|| hyperlocal::Uri::new("/var/run/docker.sock", "/containers/json").into());

#[derive(Debug)]
pub struct DockerSystem {
    running_containers: HashSet<[u8; 12]>,
}

pub struct ContainerLog {
    pub id: String,
    pub handle: JoinHandle<()>,
    pub stdout: UnboundedReceiver<Bytes>,
    pub stderr: UnboundedReceiver<Bytes>,
}

impl ContainerLog {
    pub async fn new(id: String) -> Result<Self, Box<dyn Error>> {
        let stdout_uri = hyperlocal::Uri::new(
            "/var/run/docker.sock",
            &format!("/containers/{}/logs?stdout=1&follow=1", &id),
        )
        .into();

        let mut response = UNIX_CLIENT.get(stdout_uri).await?;

        let (stdout_tx, stdout_rx) = tokio::sync::mpsc::unbounded_channel::<Bytes>();

        let stdout_handle = tokio::spawn(async move {
            while let Some(data) = response.data().await {
                match data {
                    Ok(data) => {
                        stdout_tx.send(data).unwrap();
                    }
                    Err(_err) => panic!(),
                }
            }
        });

        let start = SystemTime::now();
        let now = start
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();

        let stderr_uri = hyperlocal::Uri::new(
            "/var/run/docker.sock",
            &format!("/containers/{}/logs?stderr=1&follow=1&since={}", &id, now),
        )
        .into();

        let mut response = UNIX_CLIENT.get(stderr_uri).await?;
        let (stderr_tx, stderr_rx) = tokio::sync::mpsc::unbounded_channel::<Bytes>();

        let stderr_handle = tokio::spawn(async move {
            while let Some(data) = response.data().await {
                match data {
                    Ok(data) => {
                        stderr_tx.send(data).unwrap();
                    }
                    Err(_err) => panic!(),
                }
            }
        });

        let handle = tokio::spawn(async move {
            stdout_handle.await.unwrap();
            stderr_handle.await.unwrap();
        });

        Ok(Self {
            id,
            handle,
            stdout: stdout_rx,
            stderr: stderr_rx,
        })
    }
}

impl DockerSystem {
    // rust-analyzer.experimental.procAttrMacros
    pub async fn refresh_containers(&mut self) -> Result<(), Box<dyn Error>> {
        let mut response = UNIX_CLIENT.get(CONTAINERS_ENDPOINT.clone()).await.unwrap();
        let mut buf: Vec<u8> = Vec::with_capacity(
            (response
                .size_hint()
                .upper()
                .unwrap_or_else(|| response.size_hint().lower())) as usize,
        );

        while let Some(data) = response.data().await {
            let data = data?;
            buf.extend_from_slice(&data);
        }

        let parsed: serde_json::Value = serde_json::from_slice(&buf)?;

        let currently_running = parsed
            .as_array()
            .unwrap()
            .into_iter()
            .map(|v| {
                let bytes = &v.get("Id").unwrap().as_str().unwrap().as_bytes()[0..12];
                let mut arr = [0u8; 12];
                arr.clone_from_slice(bytes);
                arr
            })
            .collect::<HashSet<_>>();

        let new = currently_running
            .difference(&self.running_containers)
            .map(|f| f.clone())
            .collect::<Vec<_>>();

        self.running_containers.extend(new);

        let dropped = self
            .running_containers
            .difference(&currently_running)
            .map(|f| f.clone())
            .collect::<Vec<_>>();

        for drop in dropped {
            self.running_containers.remove(&drop);
        }
        Ok(())
    }

    pub fn running_containers(&self) -> Vec<String> {
        self.running_containers
            .iter()
            .map(|c| hex::encode(hex::decode(std::str::from_utf8(c).unwrap()).unwrap()))
            .collect::<Vec<_>>()
    }

    pub async fn new() -> Result<Self, Box<dyn Error>> {
        let mut s = Self {
            running_containers: Default::default(),
        };

        s.refresh_containers().await.unwrap();
        Ok(s)
    }
}

#[cfg(test)]
mod tests {

    use crate::{ContainerLog, DockerSystem};

    #[tokio::test]
    async fn list_containers_test() {
        let system = DockerSystem::new().await.unwrap();

        println!("{:#?}", system.running_containers());
    }

    #[tokio::test]
    async fn socket_open() {
        let system = DockerSystem::new().await.unwrap();

        let mut log = ContainerLog::new(system.running_containers().first().unwrap().to_string())
            .await
            .unwrap();

        while let Some(r) = log.stdout.recv().await {
            std::str::from_utf8(&r).unwrap();
            break;
        }

        log.handle.await.unwrap();
    }
}
