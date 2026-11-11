//! A release feed on localhost and keys to sign it, for the updater's tests.

use std::collections::HashMap;
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

/// A fresh key as `PUBLIC_KEY` holds one, and a signer that writes `.minisig` text.
pub fn keys() -> (String, impl Fn(&[u8]) -> String) {
    let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let key = pair.pk.to_base64();
    let sign = move |data: &[u8]| {
        let signature = minisign::sign(None, &pair.sk, data, None, None).unwrap();
        signature.into_string()
    };
    (key, sign)
}

/// Serves `latest.json`, its `.minisig` and `/package`; anything else is a 404.
pub struct Server {
    pub base: String,
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    length: Arc<AtomicBool>,
    hold: Arc<Mutex<Option<Receiver<()>>>>,
}

impl Server {
    /// A server that offers nothing yet.
    pub fn empty() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let server = Server {
            base: format!("http://{}", listener.local_addr().unwrap()),
            files: Default::default(),
            length: Arc::new(true.into()),
            hold: Default::default(),
        };
        let (files, length, hold) = (
            server.files.clone(),
            server.length.clone(),
            server.hold.clone(),
        );
        thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                // The whole request is read, or closing the socket could reset the response.
                let head: Vec<String> = BufReader::new(&stream)
                    .lines()
                    .map_while(|l| l.ok())
                    .take_while(|l| !l.is_empty())
                    .collect();
                let path = head.first().and_then(|l| l.split(' ').nth(1));
                let body = path.and_then(|p| files.lock().unwrap().get(p).cloned());
                let status = if body.is_some() {
                    "200 OK"
                } else {
                    "404 Not Found"
                };
                let body = body.unwrap_or_default();
                let len = match length.load(Ordering::Relaxed) {
                    true => format!("Content-Length: {}\r\n", body.len()),
                    false => String::new(),
                };
                let head = format!("HTTP/1.1 {status}\r\n{len}Connection: close\r\n\r\n");
                stream.write_all(head.as_bytes()).ok();
                let held = (path == Some("/package"))
                    .then(|| hold.lock().unwrap().take())
                    .flatten();
                let (first, rest) = body.split_at(if held.is_some() { body.len() / 2 } else { 0 });
                stream.write_all(first).ok();
                stream.flush().ok();
                if let Some(held) = held {
                    held.recv().ok();
                }
                stream.write_all(rest).ok();
            }
        });
        server
    }

    /// Offers `version` for this platform as `/package` with `signature`, in a `latest.json`
    /// signed by `sign`.
    pub fn start(
        sign: impl Fn(&[u8]) -> String,
        version: &str,
        package: &[u8],
        signature: &str,
    ) -> Server {
        let server = Server::empty();
        server.offer(sign, version, package, signature);
        server
    }

    /// Replaces what `start` offers.
    pub fn offer(
        &self,
        sign: impl Fn(&[u8]) -> String,
        version: &str,
        package: &[u8],
        signature: &str,
    ) {
        let platform = format!("{}-{}", env::consts::OS, env::consts::ARCH);
        let url = format!("{}/package", self.base);
        let entry = serde_json::json!({"url": url, "signature": signature});
        let latest = serde_json::json!({"version": version, "platforms": {platform: entry}});
        let latest = latest.to_string().into_bytes();
        *self.files.lock().unwrap() = HashMap::from([
            ("/latest.json.minisig".into(), sign(&latest).into_bytes()),
            ("/latest.json".into(), latest),
            ("/package".into(), package.to_vec()),
        ]);
    }

    /// Bodies end with the connection, with no length.
    pub fn without_length(&self) {
        self.length.store(false, Ordering::Relaxed);
    }

    /// The next `/package` sends its first half, then the rest once the sender sends or drops.
    pub fn hold(&self) -> Sender<()> {
        let (tx, rx) = mpsc::channel();
        *self.hold.lock().unwrap() = Some(rx);
        tx
    }
}
