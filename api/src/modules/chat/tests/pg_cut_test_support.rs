use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use sqlx::PgPool;
use sqlx::postgres::{PgPoolOptions, PgSslMode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cut {
    BeforeCommit,
    AfterCommit,
    HoldCommit,
    HoldReply,
}

#[derive(Default)]
struct Gate {
    landed: Notify,
    release: Notify,
}

pub struct PgCut {
    pub pool: PgPool,
    armed: Arc<Mutex<Option<Cut>>>,
    gate: Arc<Gate>,
}

type ToClient = Arc<tokio::sync::Mutex<Option<OwnedWriteHalf>>>;

impl PgCut {
    pub async fn start(pool: &PgPool) -> Self {
        let upstream = pool.connect_options();
        let target = format!("{}:{}", upstream.get_host(), upstream.get_port());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a proxy port");
        let port = listener.local_addr().expect("an address").port();
        let armed = Arc::new(Mutex::new(None));
        let gate = Arc::new(Gate::default());
        let (plan, gates) = (armed.clone(), gate.clone());
        tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                tokio::spawn(relay(client, target.clone(), plan.clone(), gates.clone()));
            }
        });
        let options = (*upstream)
            .clone()
            .host("127.0.0.1")
            .port(port)
            .ssl_mode(PgSslMode::Disable);
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_lazy_with(options);
        Self { pool, armed, gate }
    }

    pub fn arm(&self, cut: Cut) {
        *self.armed.lock().expect("the plan") = Some(cut);
    }

    pub fn fired(&self) -> bool {
        self.armed.lock().expect("the plan").is_none()
    }

    pub async fn landed(&self) {
        self.gate.landed.notified().await;
    }

    pub fn release(&self) {
        self.gate.release.notify_one();
    }
}

async fn relay(
    client: TcpStream,
    target: String,
    armed: Arc<Mutex<Option<Cut>>>,
    gate: Arc<Gate>,
) -> std::io::Result<()> {
    let server = TcpStream::connect(target).await?;
    let (mut from_client, to_client) = client.into_split();
    let (from_server, mut to_server) = server.into_split();
    let to_client: ToClient = Arc::new(tokio::sync::Mutex::new(Some(to_client)));
    let muted = Arc::new(AtomicBool::new(false));
    let replied = Arc::new(Notify::new());
    tokio::spawn(answers(
        from_server,
        to_client.clone(),
        muted.clone(),
        replied.clone(),
    ));
    let length = from_client.read_i32().await?;
    let mut startup = vec![0; frame(length)];
    from_client.read_exact(&mut startup).await?;
    to_server.write_i32(length).await?;
    to_server.write_all(&startup).await?;
    loop {
        let kind = from_client.read_u8().await?;
        let length = from_client.read_i32().await?;
        let mut body = vec![0; frame(length)];
        from_client.read_exact(&mut body).await?;
        let commit = kind == b'Q' && body.starts_with(b"COMMIT");
        let cut = if commit {
            armed.lock().expect("the plan").take()
        } else {
            None
        };
        let Some(cut) = cut else {
            forward(&mut to_server, kind, length, &body).await?;
            continue;
        };
        if cut == Cut::BeforeCommit {
            return Ok(());
        }
        muted.store(true, Ordering::SeqCst);
        if cut == Cut::HoldCommit {
            to_client.lock().await.take();
            drop(from_client);
            gate.release.notified().await;
        }
        forward(&mut to_server, kind, length, &body).await?;
        replied.notified().await;
        gate.landed.notify_one();
        if cut == Cut::HoldReply {
            gate.release.notified().await;
        }
        return Ok(());
    }
}

async fn forward(
    to_server: &mut OwnedWriteHalf,
    kind: u8,
    length: i32,
    body: &[u8],
) -> std::io::Result<()> {
    to_server.write_u8(kind).await?;
    to_server.write_i32(length).await?;
    to_server.write_all(body).await
}

async fn answers(
    mut from_server: OwnedReadHalf,
    to_client: ToClient,
    muted: Arc<AtomicBool>,
    replied: Arc<Notify>,
) -> std::io::Result<()> {
    let mut buffer = vec![0; 16 * 1024];
    loop {
        let read = from_server.read(&mut buffer).await?;
        if read == 0 {
            return Ok(());
        }
        if muted.load(Ordering::SeqCst) {
            replied.notify_one();
            return Ok(());
        }
        if let Some(to_client) = to_client.lock().await.as_mut() {
            to_client.write_all(&buffer[..read]).await?;
        }
    }
}

fn frame(length: i32) -> usize {
    usize::try_from(length).unwrap_or(4).saturating_sub(4)
}
