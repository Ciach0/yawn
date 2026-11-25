use super::{Error, RequestBuilder, Response};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::client::legacy::{Client, connect::HttpConnector};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

type Hyper = Client<HttpsConnector<HttpConnector>, Full<Bytes>>;

/// Makes `ring` the process-wide rustls provider, so nothing else that builds a TLS config panics
/// for lack of one
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn client() -> &'static Hyper {
    static CLIENT: OnceLock<Hyper> = OnceLock::new();
    CLIENT.get_or_init(|| {
        // the provider is given explicitly, so this works whatever else is linked in
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions")
        .with_root_certificates(rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        })
        .with_no_client_auth();
        let mut http = HttpConnector::new();
        http.enforce_http(false);
        http.set_connect_timeout(Some(Duration::from_secs(10)));
        http.set_nodelay(true);
        let https = HttpsConnectorBuilder::new()
            .with_tls_config(tls)
            .https_or_http()
            .enable_http1()
            .wrap_connector(http);
        Client::builder(hyper_util::rt::TokioExecutor::new())
            .pool_idle_timeout(Duration::from_secs(60))
            .build(https)
    })
}

pub(super) async fn send(req: RequestBuilder) -> Result<Response, Error> {
    let mut builder = hyper::Request::builder().method(req.method.as_str()).uri(req.url.as_str());
    let mut has_agent = false;
    for (n, v) in &req.headers {
        has_agent |= n.eq_ignore_ascii_case("user-agent");
        builder = builder.header(n.as_str(), v.as_str());
    }
    if !has_agent {
        builder = builder.header("user-agent", concat!("yawn/", env!("CARGO_PKG_VERSION")));
    }
    let request =
        builder.body(Full::new(Bytes::from(req.body))).map_err(|e| Error::Url(e.to_string()))?;
    let work = async {
        let res = client().request(request).await.map_err(|e| Error::Network(chain(&e)))?;
        let status = res.status().as_u16();
        let headers = res
            .headers()
            .iter()
            .map(|(n, v)| {
                (n.as_str().to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned())
            })
            .collect();
        let body = res
            .into_body()
            .collect()
            .await
            .map_err(|e| Error::Network(chain(&e)))?
            .to_bytes()
            .to_vec();
        Ok(Response { status, headers, body })
    };
    tokio::time::timeout(req.timeout, work).await.map_err(|_| Error::Timeout)?
}

fn chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        s.push_str(": ");
        s.push_str(&c.to_string());
        cur = c.source();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[tokio::test]
    async fn talks_plain_http_and_reads_json() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut s, _) = l.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = s.read(&mut buf).await.unwrap();
            let seen = String::from_utf8_lossy(&buf[..n]).to_lowercase();
            let body = format!(
                "{{\"auth\":{},\"post\":{}}}",
                seen.contains("authorization: bearer tok"),
                seen.starts_with("post /p")
            );
            s.write_all(format!("HTTP/1.1 201 Created\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        });
        let res = post(format!("http://{addr}/p"))
            .bearer("tok")
            .form(&[("a", "b")])
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 201);
        assert_eq!(res.header("Content-Type"), Some("application/json"));
        let v: serde_json::Value = res.json().unwrap();
        assert_eq!((v["auth"].as_bool(), v["post"].as_bool()), (Some(true), Some(true)));
    }

    #[tokio::test]
    async fn refuses_other_schemes_and_reports_a_dead_port() {
        assert!(matches!(get("ftp://x").send().await, Err(Error::Url(_))));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        drop(l);
        assert!(matches!(get(format!("http://{addr}/")).send().await, Err(Error::Network(_))));
    }

    #[tokio::test]
    #[ignore]
    async fn real_https() {
        let res = get("https://discord.com/api/v10/gateway").send().await.unwrap();
        assert_eq!(res.status(), 200);
        assert!(res.text().unwrap().contains("wss://"));
    }
}
