use hyper::{HeaderMap, Method};
use runtime_api::{
    proof::{ProofContext, create_server_proof, decode_hex, encode_hex},
    token::{SecretToken, init_private_token, load_private_token},
};
use runtime_cli::client::{RequestBody, VerifiedConnection};
use std::{net::SocketAddr, path::Path, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use uuid::Uuid;

async fn read_head(socket: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    while bytes.len() < 32 * 1024 {
        match tokio::time::timeout(Duration::from_secs(3), socket.read(&mut byte)).await {
            Ok(Ok(1)) => {
                bytes.push(byte[0]);
                if bytes.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            _ => break,
        }
    }
    String::from_utf8(bytes).unwrap_or_default()
}
fn challenge(head: &str) -> [u8; 32] {
    head.lines()
        .find_map(|line| {
            let (k, v) = line.split_once(':')?;
            (k.eq_ignore_ascii_case("x-nexa-server-challenge"))
                .then(|| decode_hex::<32>(v.trim()).unwrap())
        })
        .unwrap()
}
fn response(
    token: &SecretToken,
    instance: Uuid,
    nonce: [u8; 32],
    client: SocketAddr,
    server: SocketAddr,
) -> String {
    let proof = create_server_proof(
        token,
        &ProofContext {
            instance_id: *instance.as_bytes(),
            nonce,
            client,
            server,
        },
    );
    format!(
        "HTTP/1.1 200 OK\r\nX-Nexa-Instance-ID: {instance}\r\nX-Nexa-Protocol-Version: 1\r\nX-Nexa-Server-Proof: {}\r\nCache-Control: no-store\r\nContent-Length: 2\r\n\r\n{{}}",
        encode_hex(&proof)
    )
}
fn tokens(root: &Path) -> (SecretToken, SecretToken) {
    let root = root.join("private");
    let first = init_private_token(&root).unwrap();
    (first, load_private_token(&root).unwrap())
}

#[tokio::test]
async fn only_verified_same_connection_receives_bearer() {
    let temp = tempfile::tempdir().unwrap();
    let (client_token, server_token) = tokens(temp.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let id = Uuid::new_v4();
    let server = tokio::spawn(async move {
        let (mut socket, peer) = listener.accept().await.unwrap();
        let first = read_head(&mut socket).await;
        assert!(!first.to_ascii_lowercase().contains("authorization:"));
        let proof = response(
            &server_token,
            id,
            challenge(&first),
            peer,
            socket.local_addr().unwrap(),
        );
        socket.write_all(proof.as_bytes()).await.unwrap();
        let second = read_head(&mut socket).await;
        assert!(
            second
                .to_ascii_lowercase()
                .contains("authorization: bearer ")
        );
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
            .await
            .unwrap();
    });
    let mut client = VerifiedConnection::connect(address, id, client_token)
        .await
        .unwrap();
    client
        .json(Method::GET, "/runtime/status", None)
        .await
        .unwrap();
    server.await.unwrap();
}
#[tokio::test]
async fn forged_redirect_missing_duplicate_oversized_and_endpoint_proofs_send_no_auth() {
    for variant in [
        "echo",
        "missing",
        "duplicate",
        "redirect",
        "oversized_header",
        "oversized_body",
        "wrong_instance",
        "wrong_version",
        "altered_client",
        "altered_server",
        "replayed_nonce",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let (client_token, server_token) = tokens(temp.path());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let id = Uuid::new_v4();
        let server = tokio::spawn(async move {
            let (mut socket, peer) = listener.accept().await.unwrap();
            let first = read_head(&mut socket).await;
            assert!(!first.to_ascii_lowercase().contains("authorization:"));
            let nonce = challenge(&first);
            let mut proof = response(&server_token, id, nonce, peer, socket.local_addr().unwrap());
            match variant {
                "echo" => {
                    let correct = encode_hex(&create_server_proof(
                        &server_token,
                        &ProofContext {
                            instance_id: *id.as_bytes(),
                            nonce,
                            client: peer,
                            server: address,
                        },
                    ));
                    proof = proof.replace(&correct, &encode_hex(&nonce));
                }
                "missing" => {
                    proof = proof
                        .lines()
                        .filter(|x| !x.starts_with("X-Nexa-Server-Proof"))
                        .collect::<Vec<_>>()
                        .join("\r\n")
                }
                "duplicate" => {
                    proof = proof.replace(
                        "Cache-Control:",
                        &format!("X-Nexa-Server-Proof: {}\r\nCache-Control:", "0".repeat(64)),
                    )
                }
                "redirect" => proof = proof.replace("200 OK", "302 Found"),
                "oversized_header" => {
                    proof = proof.replace(
                        "Cache-Control:",
                        &format!("X-Overflow: {}\r\nCache-Control:", "a".repeat(20000)),
                    )
                }
                "oversized_body" => {
                    proof = proof.replace(
                        "Content-Length: 2\r\n\r\n{}",
                        &format!("Content-Length: 5000\r\n\r\n{}", "x".repeat(5000)),
                    )
                }
                "wrong_instance" => {
                    proof = proof.replace(&id.to_string(), &Uuid::new_v4().to_string())
                }
                "wrong_version" => proof = proof.replace("Version: 1", "Version: 2"),
                "altered_client" => {
                    proof = response(
                        &server_token,
                        id,
                        nonce,
                        SocketAddr::new(peer.ip(), peer.port().wrapping_add(1)),
                        address,
                    )
                }
                "altered_server" => {
                    proof = response(
                        &server_token,
                        id,
                        nonce,
                        peer,
                        SocketAddr::new(address.ip(), address.port().wrapping_add(1)),
                    )
                }
                "replayed_nonce" => proof = response(&server_token, id, [9; 32], peer, address),
                _ => unreachable!(),
            }
            let _ = socket.write_all(proof.as_bytes()).await;
            let second = read_head(&mut socket).await;
            assert!(
                !second.to_ascii_lowercase().contains("authorization:"),
                "{variant}"
            );
        });
        assert!(
            VerifiedConnection::connect(address, id, client_token)
                .await
                .is_err(),
            "{variant}"
        );
        server.await.unwrap();
    }
}
#[tokio::test]
async fn live_relay_cannot_reuse_backend_socket_proof() {
    let temp = tempfile::tempdir().unwrap();
    let (client_token, server_token) = tokens(temp.path());
    let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let backend_address = backend.local_addr().unwrap();
    let relay = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay_address = relay.local_addr().unwrap();
    let id = Uuid::new_v4();
    let backend_task = tokio::spawn(async move {
        let (mut socket, peer) = backend.accept().await.unwrap();
        let first = read_head(&mut socket).await;
        assert!(!first.to_ascii_lowercase().contains("authorization:"));
        socket
            .write_all(
                response(&server_token, id, challenge(&first), peer, backend_address).as_bytes(),
            )
            .await
            .unwrap();
    });
    let relay_task = tokio::spawn(async move {
        let (mut front, _) = relay.accept().await.unwrap();
        let first = read_head(&mut front).await;
        assert!(!first.to_ascii_lowercase().contains("authorization:"));
        let mut back = TcpStream::connect(backend_address).await.unwrap();
        back.write_all(first.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        back.read_to_end(&mut response).await.unwrap();
        front.write_all(&response).await.unwrap();
        assert!(
            !read_head(&mut front)
                .await
                .to_ascii_lowercase()
                .contains("authorization:")
        );
    });
    assert!(
        VerifiedConnection::connect(relay_address, id, client_token)
            .await
            .is_err()
    );
    backend_task.await.unwrap();
    relay_task.await.unwrap();
}
#[tokio::test]
async fn closure_after_proof_never_reconnects_to_reused_endpoint() {
    let temp = tempfile::tempdir().unwrap();
    let (client_token, server_token) = tokens(temp.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let id = Uuid::new_v4();
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, peer) = listener.accept().await.unwrap();
        let first = read_head(&mut socket).await;
        socket
            .write_all(response(&server_token, id, challenge(&first), peer, address).as_bytes())
            .await
            .unwrap();
        socket.shutdown().await.unwrap();
        drop(socket);
        drop(listener);
        let _ = closed_tx.send(());
    });
    let mut client = VerifiedConnection::connect(address, id, client_token)
        .await
        .unwrap();
    closed_rx.await.unwrap();
    server.await.unwrap();
    let attacker_socket = tokio::net::TcpSocket::new_v4().unwrap();
    attacker_socket.set_reuseaddr(true).unwrap();
    attacker_socket.bind(address).unwrap();
    let attacker = attacker_socket.listen(16).unwrap();
    assert!(
        client
            .request(
                Method::GET,
                "/runtime/status",
                RequestBody::fixed(Vec::new()),
                HeaderMap::new()
            )
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), attacker.accept())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn recorded_valid_mac_cannot_be_replayed_to_new_connection() {
    let temp = tempfile::tempdir().unwrap();
    let (client_token, server_token) = tokens(temp.path());
    let second_token = load_private_token(&temp.path().join("private")).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let id = Uuid::new_v4();
    let server = tokio::spawn(async move {
        let (mut first, peer) = listener.accept().await.unwrap();
        let head = read_head(&mut first).await;
        let captured = response(&server_token, id, challenge(&head), peer, address);
        first.write_all(captured.as_bytes()).await.unwrap();
        let _ = read_head(&mut first).await;
        drop(first);
        let (mut second, _) = listener.accept().await.unwrap();
        let head = read_head(&mut second).await;
        assert!(!head.to_ascii_lowercase().contains("authorization:"));
        second.write_all(captured.as_bytes()).await.unwrap();
        let rest = read_head(&mut second).await;
        assert!(!rest.to_ascii_lowercase().contains("authorization:"));
    });
    let first = VerifiedConnection::connect(address, id, client_token)
        .await
        .unwrap();
    drop(first);
    assert!(
        VerifiedConnection::connect(address, id, second_token)
            .await
            .is_err()
    );
    server.await.unwrap();
}
#[tokio::test]
async fn proof_timeout_closes_socket_without_sending_bearer() {
    let temp = tempfile::tempdir().unwrap();
    let (client_token, _) = tokens(temp.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let id = Uuid::new_v4();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let head = read_head(&mut socket).await;
        assert!(!head.to_ascii_lowercase().contains("authorization:"));
        let mut rest = Vec::new();
        tokio::time::timeout(Duration::from_secs(7), socket.read_to_end(&mut rest))
            .await
            .unwrap()
            .unwrap();
        assert!(
            !String::from_utf8_lossy(&rest)
                .to_ascii_lowercase()
                .contains("authorization:")
        );
    });
    assert!(
        VerifiedConnection::connect(address, id, client_token)
            .await
            .is_err()
    );
    server.await.unwrap();
}
