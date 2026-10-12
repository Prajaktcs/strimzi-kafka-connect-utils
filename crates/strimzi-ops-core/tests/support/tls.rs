use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use httpmock::MockServer;
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose,
};
use rustls::pki_types::PrivatePkcs8KeyDer;
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use tempfile::TempDir;

/// Real TLS front end for HTTP mocks; collectors never see the plaintext backend.
pub struct TlsMockServer {
    mock: MockServer,
    address: SocketAddr,
    ca_path: PathBuf,
    _directory: TempDir,
    stop: Arc<AtomicBool>,
    listener: Option<JoinHandle<()>>,
}

impl TlsMockServer {
    pub fn start() -> Self {
        Self::start_with_names(vec!["localhost".to_owned(), "127.0.0.1".to_owned()])
    }

    pub fn start_with_names(names: Vec<String>) -> Self {
        let mock = MockServer::start();
        let backend = *mock.address();
        Self::with_backend(mock, backend, names)
    }

    /// Used by concurrency tests with independently gated plaintext backends.
    #[allow(dead_code)]
    pub fn start_forwarding_to(backend: SocketAddr) -> Self {
        Self::with_backend(
            MockServer::start(),
            backend,
            vec!["localhost".to_owned(), "127.0.0.1".to_owned()],
        )
    }

    fn with_backend(mock: MockServer, backend: SocketAddr, names: Vec<String>) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "metrics test CA");
        let ca_key = KeyPair::generate().unwrap();
        let ca = ca_params.self_signed(&ca_key).unwrap();
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(names).unwrap();
        params
            .distinguished_name
            .push(DnType::CommonName, "metrics test exporter");
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let cert = params.signed_by(&key, &ca, &ca_key).unwrap();
        let ca_path = directory.path().join("ca.crt");
        std::fs::write(&ca_path, ca.pem()).unwrap();
        let config =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(
                    vec![cert.der().clone()],
                    PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
                )
                .unwrap();
        let config = Arc::new(config);
        let socket = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = socket.local_addr().unwrap();
        socket.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let listener = thread::spawn(move || {
            let mut requests = Vec::new();
            while !stopped.load(Ordering::Relaxed) {
                match socket.accept() {
                    Ok((socket, _)) => {
                        let config = Arc::clone(&config);
                        requests.push(thread::spawn(move || {
                            // A failed trust/hostname handshake is expected in negative tests.
                            let _ = forward(socket, backend, config);
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("cannot accept TLS fixture request: {error}"),
                }
            }
            for request in requests {
                request.join().unwrap();
            }
        });
        Self {
            mock,
            address,
            ca_path,
            _directory: directory,
            stop,
            listener: Some(listener),
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url())
    }

    pub fn base_url(&self) -> String {
        format!("https://{}", self.address)
    }

    pub fn ca_path(&self) -> &Path {
        &self.ca_path
    }
}

impl Deref for TlsMockServer {
    type Target = MockServer;

    fn deref(&self) -> &Self::Target {
        &self.mock
    }
}

impl Drop for TlsMockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(listener) = self.listener.take() {
            listener.join().unwrap();
        }
    }
}

fn forward(
    socket: TcpStream,
    backend: SocketAddr,
    config: Arc<ServerConfig>,
) -> std::io::Result<()> {
    socket.set_nonblocking(false)?;
    let timeout = Some(Duration::from_secs(8));
    socket.set_read_timeout(timeout)?;
    socket.set_write_timeout(timeout)?;
    let connection = ServerConnection::new(config).map_err(std::io::Error::other)?;
    let mut tls = StreamOwned::new(connection, socket);
    let mut request = Vec::new();
    let mut byte = [0];
    while !request.ends_with(b"\r\n\r\n") {
        if request.len() >= 16 * 1024 || tls.read(&mut byte)? == 0 {
            return Err(std::io::Error::other("invalid TLS fixture request"));
        }
        request.push(byte[0]);
    }
    let request = String::from_utf8(request).map_err(std::io::Error::other)?;
    let mut upstream = TcpStream::connect_timeout(&backend, Duration::from_secs(3))?;
    upstream.set_read_timeout(timeout)?;
    upstream.set_write_timeout(timeout)?;
    // Close each upstream connection so raw response forwarding has a defined EOF.
    for line in request.split("\r\n").filter(|line| !line.is_empty()) {
        if !line.to_ascii_lowercase().starts_with("connection:") {
            upstream.write_all(line.as_bytes())?;
            upstream.write_all(b"\r\n")?;
        }
    }
    upstream.write_all(b"Connection: close\r\n\r\n")?;
    let mut response = Vec::new();
    while !response.ends_with(b"\r\n\r\n") {
        if response.len() >= 16 * 1024 || upstream.read(&mut byte)? == 0 {
            return Err(std::io::Error::other("invalid TLS fixture response"));
        }
        response.push(byte[0]);
    }
    let response = String::from_utf8(response).map_err(std::io::Error::other)?;
    for line in response.split("\r\n").filter(|line| !line.is_empty()) {
        if !line.to_ascii_lowercase().starts_with("connection:") {
            tls.write_all(line.as_bytes())?;
            tls.write_all(b"\r\n")?;
        }
    }
    // This fixture handles one request per TLS connection; never advertise keep-alive.
    tls.write_all(b"Connection: close\r\n\r\n")?;
    std::io::copy(&mut upstream, &mut tls)?;
    tls.flush()?;
    tls.conn.send_close_notify();
    tls.flush()?;
    Ok(())
}
