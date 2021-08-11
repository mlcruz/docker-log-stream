use std::{borrow::Cow, collections::HashSet, error::Error};

use hyper::{body::HttpBody, Client};
use hyperlocal::{UnixClientExt, UnixConnector, Uri};
use once_cell::sync::Lazy;

static UNIX_CLIENT: Lazy<Client<UnixConnector>> = Lazy::new(|| Client::unix());
static CONTAINERS_ENDPOINT: Lazy<hyper::Uri> =
    Lazy::new(|| hyperlocal::Uri::new("/var/run/docker.sock", "/containers/json").into());

#[derive(Debug)]
pub struct DockerSystem {
    running_containers: HashSet<Vec<u8>>,
}

impl DockerSystem {
    // rust-analyzer.experimental.procAttrMacros
    async fn refresh_containers(&mut self) -> Result<(), Box<dyn Error>> {
        let mut response = UNIX_CLIENT.get(CONTAINERS_ENDPOINT.clone()).await.unwrap();
        dbg!(response.size_hint());
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
            .map(|v| hex::decode(v.get("Id").unwrap().as_str().unwrap()).unwrap())
            .collect::<HashSet<_>>();

        let new = currently_running
            .difference(&self.running_containers)
            .map(|f| f.to_vec())
            .collect::<Vec<_>>();

        self.running_containers.extend(new);

        let dropped = self
            .running_containers
            .difference(&currently_running)
            .map(|f| f.to_vec())
            .collect::<Vec<_>>();

        for drop in dropped {
            self.running_containers.remove(&drop);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use hyper::{
        body::{Bytes, HttpBody},
        Client,
    };
    use hyperlocal::{UnixClientExt, Uri};

    use crate::DockerSystem;

    #[tokio::test]
    async fn list_containers_test() {
        let mut system = DockerSystem {
            running_containers: HashSet::new(),
        };

        system.refresh_containers().await;
        dbg!(system);
    }

    #[tokio::test]
    async fn socket_open() {
        let url = Uri::new(
            "/var/run/docker.sock",
            "/containers/3fe658779d14/logs?stdout=1&stderr=1&follow=1",
        )
        .into();

        let client = Client::unix();

        let mut response = client.get(url).await.unwrap();
        let foo = response.into_body().data();
    }
}
