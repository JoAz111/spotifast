//! Spotify Soloist control over the private Unix socket exposed by its runtime.
//!
//! This transport carries no audio and makes no claim about the output format.
//! The wire protocol is documented at
//! <https://developer.spotify.com/documentation/soloist/reference/websocket-api>.

use std::path::PathBuf;
use std::sync::Arc;

use crate::api::models::Queue;
use crate::player::{LoadSpec, LocalState, PlayerCommand};

pub enum Update {
    State(LocalState),
    Ready(bool),
    Queue {
        seq: u64,
        queue: Box<Queue>,
    },
    /// Activation was observed. The caller must preserve this complete request
    /// when it dispatches playback through the user's Spotify Web API grant.
    Load {
        spec: LoadSpec,
        /// The caller's account token captured when this load was admitted.
        generation: u64,
    },
    Error(String),
}

pub type Notify = Arc<dyn Fn(Update) + Send + Sync>;

#[cfg(unix)]
pub use unix::Handle;

#[cfg(not(unix))]
pub struct Handle;

#[cfg(not(unix))]
impl Handle {
    pub fn start(_path: PathBuf, notify: Notify) -> Self {
        notify(Update::Ready(false));
        notify(Update::Error(unsupported()));
        Self
    }

    pub fn command(&self, _command: PlayerCommand) -> Result<(), String> {
        Err(unsupported())
    }

    pub fn command_for_session(
        &self,
        _command: PlayerCommand,
        _generation: u64,
    ) -> Result<(), String> {
        Err(unsupported())
    }

    pub fn request_queue(&self, _seq: u64) -> Result<(), String> {
        Err(unsupported())
    }

    pub fn pause(&self) -> Result<(), String> {
        Err(unsupported())
    }
}

#[cfg(not(unix))]
fn unsupported() -> String {
    "Soloist requires a private Unix socket on this platform.".into()
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::collections::VecDeque;
    use std::fs::Metadata;
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use futures_util::{SinkExt, StreamExt};
    use serde::{Deserialize, Serialize};
    use tokio::net::UnixStream;
    use tokio::sync::{mpsc, watch};
    use tokio::task::JoinHandle;
    use tokio::time::{interval, sleep, timeout};
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
    use tokio_tungstenite::{WebSocketStream, client_async_with_config};

    use crate::api::models::{Album, ArtistRef, Episode, Image, PlayableItem, Show, Track};
    use crate::player::{LocalTrack, Playback, RepeatMode};

    const LIMIT: usize = 32;
    const MAX_MESSAGE: usize = 1024 * 1024;
    const QUEUE_LIMIT: usize = 80;
    const DEADLINE: Duration = Duration::from_secs(5);
    const WRITE_DEADLINE: Duration = Duration::from_secs(2);
    const DISCONNECTED: &str =
        "Soloist is not ready. Check the local runtime and its Spotify login.";
    const BUSY: &str = "Soloist has too many pending commands. Wait for playback to catch up.";

    #[derive(Default)]
    struct Admission {
        /// Serializes only synchronous socket-epoch/admission transitions.
        /// No socket I/O, notification callback, or await holds this lock.
        transition: std::sync::Mutex<()>,
        ready: AtomicBool,
        pausing: AtomicBool,
        connection_generation: AtomicU64,
        generation: AtomicU64,
        load_generation: AtomicU64,
        outstanding: AtomicUsize,
        queue: AtomicBool,
        load: AtomicBool,
    }

    #[derive(Clone, Copy, Default)]
    enum TicketKind {
        #[default]
        Control,
        Queue,
        Load,
    }

    struct Ticket {
        admission: Arc<Admission>,
        cost: usize,
        kind: TicketKind,
    }

    impl Ticket {
        fn reserve(admission: &Arc<Admission>, cost: usize) -> Result<Self, String> {
            admission
                .outstanding
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                    (current + cost <= LIMIT).then_some(current + cost)
                })
                .map_err(|_| BUSY.to_string())?;
            Ok(Self {
                admission: Arc::clone(admission),
                cost,
                kind: TicketKind::Control,
            })
        }

        fn split(&mut self) -> Self {
            debug_assert!(self.cost > 0);
            self.cost -= 1;
            Self {
                admission: Arc::clone(&self.admission),
                cost: 1,
                kind: TicketKind::Control,
            }
        }
    }

    impl Drop for Ticket {
        fn drop(&mut self) {
            self.admission
                .outstanding
                .fetch_sub(self.cost, Ordering::AcqRel);
            match self.kind {
                TicketKind::Queue => self.admission.queue.store(false, Ordering::Release),
                TicketKind::Load => self.admission.load.store(false, Ordering::Release),
                TicketKind::Control => {}
            }
        }
    }

    enum Request {
        Control(PlayerCommand),
        Queue(u64),
    }

    struct Pending {
        generation: u64,
        load_generation: u64,
        session_generation: u64,
        request: Request,
        ticket: Ticket,
    }

    pub struct Handle {
        sender: mpsc::Sender<Pending>,
        pause_sender: watch::Sender<u64>,
        admission: Arc<Admission>,
        task: Option<JoinHandle<()>>,
    }

    impl Handle {
        /// Start on the caller's Tokio runtime. No socket or network work runs
        /// on the UI thread. A missing runtime is reported without panicking.
        pub fn start(path: PathBuf, notify: Notify) -> Self {
            let (sender, receiver) = mpsc::channel(LIMIT);
            let (pause_sender, pauses) = watch::channel(0);
            let admission = Arc::new(Admission::default());
            notify(Update::Ready(false));
            let task = match tokio::runtime::Handle::try_current() {
                Ok(runtime) => {
                    Some(runtime.spawn(run(path, receiver, pauses, Arc::clone(&admission), notify)))
                }
                Err(_) => {
                    notify(Update::Error(
                        "Soloist control could not start on the playback runtime.".into(),
                    ));
                    None
                }
            };
            Self {
                sender,
                pause_sender,
                admission,
                task,
            }
        }

        pub fn command(&self, command: PlayerCommand) -> Result<(), String> {
            self.command_for_session(command, 0)
        }

        /// Preserve the caller's account token across receiver activation.
        /// Reading an external current token when activation completes could
        /// incorrectly relabel an earlier account's playback request.
        pub fn command_for_session(
            &self,
            command: PlayerCommand,
            session_generation: u64,
        ) -> Result<(), String> {
            validate_command(&command)?;
            let generation = self.admission.generation.load(Ordering::Acquire);
            let load_generation = self.admission.load_generation.load(Ordering::Acquire);
            if !self.admission.ready.load(Ordering::Acquire)
                || self.admission.pausing.load(Ordering::Acquire)
            {
                return Err(DISCONNECTED.into());
            }
            let cost = if matches!(command, PlayerCommand::Repeat(_)) {
                2
            } else {
                1
            };
            let mut ticket = Ticket::reserve(&self.admission, cost)?;
            if matches!(command, PlayerCommand::Load(_)) {
                self.admission
                    .load
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .map_err(|_| "Soloist is still activating the previous playback request.")?;
                ticket.kind = TicketKind::Load;
            }
            self.enqueue(
                generation,
                load_generation,
                session_generation,
                Request::Control(command),
                ticket,
            )
        }

        pub fn request_queue(&self, seq: u64) -> Result<(), String> {
            let generation = self.admission.generation.load(Ordering::Acquire);
            if !self.admission.ready.load(Ordering::Acquire) {
                return Err(DISCONNECTED.into());
            }
            let mut ticket = Ticket::reserve(&self.admission, 1)?;
            self.admission
                .queue
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .map_err(|_| "Soloist already has a queue request pending.")?;
            ticket.kind = TicketKind::Queue;
            self.enqueue(generation, 0, 0, Request::Queue(seq), ticket)
        }

        /// Sign-out must pause explicitly and cancel activation even when the
        /// receiver's current playback status is paused or still arriving.
        /// Its dedicated latest-value signal cannot be blocked by ordinary
        /// command saturation. Advancing admission first invalidates all
        /// queued mutations before the async transport observes the signal.
        pub fn pause(&self) -> Result<(), String> {
            let connection_generation = {
                let _transition = self
                    .admission
                    .transition
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                // Capture the immutable socket epoch while closing admission.
                // Authentication changes cancel mutations on the same socket,
                // while a delayed pause cannot reach a replacement socket.
                let connection_generation =
                    self.admission.connection_generation.load(Ordering::Acquire);
                let was_pausing = self.admission.pausing.swap(true, Ordering::AcqRel);
                let was_ready = self.admission.ready.swap(false, Ordering::AcqRel);
                self.admission.generation.fetch_add(1, Ordering::AcqRel);
                self.admission
                    .load_generation
                    .fetch_add(1, Ordering::AcqRel);
                if !was_ready && !was_pausing {
                    self.admission.pausing.store(false, Ordering::Release);
                    return Err(DISCONNECTED.into());
                }
                connection_generation
            };
            self.pause_sender
                .send(connection_generation)
                .map_err(|_| DISCONNECTED.into())
        }

        fn enqueue(
            &self,
            generation: u64,
            load_generation: u64,
            session_generation: u64,
            request: Request,
            ticket: Ticket,
        ) -> Result<(), String> {
            self.sender
                .try_send(Pending {
                    generation,
                    load_generation,
                    session_generation,
                    request,
                    ticket,
                })
                .map_err(|error| match error {
                    mpsc::error::TrySendError::Full(_) => BUSY.into(),
                    mpsc::error::TrySendError::Closed(_) => DISCONNECTED.into(),
                })
        }
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            self.admission.ready.store(false, Ordering::Release);
            if let Some(task) = self.task.take() {
                task.abort();
            }
        }
    }

    fn validate_command(command: &PlayerCommand) -> Result<(), String> {
        match command {
            PlayerCommand::ClearQueue => {
                return Err("Soloist does not expose a command to clear the manual queue.".into());
            }
            PlayerCommand::AddToQueue(uri) if spotify_id(uri, "track").is_none() => {
                return Err("Soloist can queue only Spotify track URIs.".into());
            }
            PlayerCommand::Load(spec) => {
                let bytes = spec.uris.iter().map(String::len).sum::<usize>()
                    + spec.context_uri.as_ref().map_or(0, String::len)
                    + spec.offset_uri.as_ref().map_or(0, String::len);
                if bytes > MAX_MESSAGE {
                    return Err("The Soloist playback request is too large.".into());
                }
                if spec.context_uri.as_ref().is_none_or(String::is_empty) && spec.uris.is_empty() {
                    return Err("The Soloist playback request has no music to play.".into());
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn current_uid() -> u32 {
        unsafe extern "C" {
            fn geteuid() -> std::os::raw::c_uint;
        }
        // POSIX geteuid has no arguments and cannot fail. Supported Unix
        // targets (macOS and Linux) use an unsigned 32-bit uid_t.
        unsafe { geteuid() }
    }

    fn validate_socket(path: &Path, uid: u32) -> Result<Metadata, String> {
        if !path.is_absolute() {
            return Err("The Soloist socket path must be absolute.".into());
        }
        let parent = path
            .parent()
            .ok_or("The Soloist socket requires a private parent directory.")?;
        let directory = std::fs::symlink_metadata(parent)
            .map_err(|_| "The Soloist socket's private directory is unavailable.")?;
        if !directory.is_dir() || directory.uid() != uid || directory.mode() & 0o7777 != 0o700 {
            return Err(
                "The Soloist socket directory must belong to this user and have mode 0700.".into(),
            );
        }
        let socket = std::fs::symlink_metadata(path)
            .map_err(|_| "The Soloist socket is unavailable. Start its local runtime.")?;
        if !socket.file_type().is_socket() || socket.uid() != uid || socket.mode() & 0o7777 != 0o600
        {
            return Err("The Soloist socket must belong to this user and have mode 0600.".into());
        }
        Ok(socket)
    }

    type Socket = WebSocketStream<UnixStream>;

    async fn connect(path: &Path) -> Result<Socket, String> {
        let uid = current_uid();
        let before = validate_socket(path, uid)?;
        let stream = timeout(DEADLINE, UnixStream::connect(path))
            .await
            .map_err(|_| "Connecting to the Soloist runtime timed out.")?
            .map_err(|_| "Could not connect to the Soloist runtime's private socket.")?;
        let peer = stream
            .peer_cred()
            .map_err(|_| "Could not verify the Soloist runtime's socket credentials.")?;
        if peer.uid() != uid {
            return Err("The Soloist runtime socket belongs to another user.".into());
        }
        let after = validate_socket(path, uid)?;
        if before.dev() != after.dev() || before.ino() != after.ino() {
            return Err("The Soloist socket changed while connecting. Try again.".into());
        }
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_MESSAGE))
            .max_frame_size(Some(MAX_MESSAGE));
        // The URI is only an HTTP handshake target on this already connected
        // Unix stream. No name resolution, proxy, or TCP connection is used.
        let (socket, _) = timeout(
            DEADLINE,
            client_async_with_config("ws://localhost/", stream, Some(config)),
        )
        .await
        .map_err(|_| "The Soloist socket handshake timed out.")?
        .map_err(|_| "The Soloist runtime did not accept its socket handshake.")?;
        Ok(socket)
    }

    async fn run(
        path: PathBuf,
        mut receiver: mpsc::Receiver<Pending>,
        mut pauses: watch::Receiver<u64>,
        admission: Arc<Admission>,
        notify: Notify,
    ) {
        let mut model = Model::default();
        let mut delay = Duration::from_millis(250);
        let mut last_error = None;
        loop {
            let outcome = match connect(&path).await {
                Ok(mut socket) => {
                    let started = Instant::now();
                    let result = connected(
                        &mut socket,
                        &mut receiver,
                        &mut pauses,
                        &admission,
                        &notify,
                        &mut model,
                    )
                    .await;
                    if started.elapsed() > Duration::from_secs(30) {
                        delay = Duration::from_millis(250);
                        last_error = None;
                    }
                    result
                }
                Err(error) => Err(error),
            };
            let refreshing = matches!(&outcome, Ok(ConnectionExit::Refresh));
            invalidate(&admission, &notify, &mut model, true);
            // An ambiguous pause belongs to the closed connection. Ordinary
            // commands and the priority signal are never replayed on a new one.
            pauses.borrow_and_update();
            match outcome {
                Ok(ConnectionExit::Stop) => return,
                Ok(ConnectionExit::Refresh) => {
                    delay = Duration::from_millis(250);
                    last_error = None;
                }
                Err(error) => {
                    if last_error.as_ref() != Some(&error) {
                        notify(Update::Error(error.clone()));
                        last_error = Some(error);
                    }
                }
            }
            // Disconnected mutations are discarded, including requests racing
            // with the readiness change. Nothing is replayed after reconnect.
            let pause = sleep(delay);
            tokio::pin!(pause);
            loop {
                tokio::select! {
                    _ = &mut pause => break,
                    changed = pauses.changed() => {
                        if changed.is_err() { return; }
                        pauses.borrow_and_update();
                    },
                    pending = receiver.recv() => match pending {
                        Some(_) if !refreshing => notify(Update::Error(DISCONNECTED.into())),
                        Some(_) => {},
                        None => return,
                    },
                }
            }
            delay = (delay * 2).min(Duration::from_secs(8));
        }
    }

    fn invalidate(
        admission: &Admission,
        notify: &Notify,
        model: &mut Model,
        connection_closed: bool,
    ) {
        {
            let _transition = admission
                .transition
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            admission.ready.store(false, Ordering::Release);
            admission.generation.fetch_add(1, Ordering::AcqRel);
            if connection_closed {
                admission.pausing.store(false, Ordering::Release);
                admission
                    .connection_generation
                    .fetch_add(1, Ordering::AcqRel);
            }
        }
        model.disconnect();
        notify(Update::Ready(false));
        notify(Update::State(model.state.clone()));
    }

    struct Control {
        name: &'static str,
        sent: Instant,
        _ticket: Option<Ticket>,
    }

    struct QueueRequest {
        seq: u64,
        sent: Instant,
        _ticket: Ticket,
    }

    struct LoadRequest {
        spec: LoadSpec,
        load_generation: u64,
        session_generation: u64,
        sent: Instant,
        _ticket: Ticket,
    }

    enum ConnectionExit {
        Stop,
        Refresh,
    }

    async fn connected(
        socket: &mut Socket,
        receiver: &mut mpsc::Receiver<Pending>,
        pauses: &mut watch::Receiver<u64>,
        admission: &Arc<Admission>,
        notify: &Notify,
        model: &mut Model,
    ) -> Result<ConnectionExit, String> {
        let mut controls: VecDeque<Control> = VecDeque::new();
        let mut queue: Option<QueueRequest> = None;
        let mut load: Option<LoadRequest> = None;
        let mut tick = interval(Duration::from_millis(250));
        let mut received = Instant::now();
        let mut pinged = received;
        let mut snapshot_deadline = Some(received);
        let mut priority_pause: Option<Instant> = None;
        let connection_generation = admission.connection_generation.load(Ordering::Acquire);
        loop {
            tokio::select! {
                biased;
                changed = pauses.changed() => {
                    if changed.is_err() { return Ok(ConnectionExit::Stop); }
                    let pause_connection = *pauses.borrow_and_update();
                    if pause_connection != connection_generation {
                        continue;
                    }
                    controls.clear();
                    queue = None;
                    load = None;
                    if priority_pause.is_none() {
                        send(socket, &WireCommand::Pause).await?;
                        // Admission stays closed until a fresh connection and
                        // state snapshot. Old named acknowledgements cannot be
                        // mistaken for new-account mutation acknowledgements.
                        priority_pause = Some(Instant::now());
                    }
                }
                incoming = socket.next() => {
                    received = Instant::now();
                    let message = incoming
                        .ok_or("The Soloist runtime socket closed. Pending commands were not retried.")?
                        .map_err(|_| "The Soloist runtime socket failed. Pending commands were not retried.")?;
                    let text = match message {
                        Message::Text(text) => text,
                        Message::Close(_) => return Err("The Soloist runtime socket closed. Pending commands were not retried.".into()),
                        Message::Ping(_) | Message::Pong(_) => continue,
                        _ => return Err("Soloist sent an unsupported socket message.".into()),
                    };
                    let event: Event = serde_json::from_str(text.as_ref())
                        .map_err(|_| "Soloist sent an invalid playback message.")?;
                    match event {
                        Event::CommandResult { command } => {
                            if command == "pause" && priority_pause.is_some() {
                                return Ok(ConnectionExit::Refresh);
                            }
                            if let Some(index) = controls.iter().position(|control| control.name == command) {
                                controls.remove(index);
                            }
                        }
                        Event::Error => {
                            return Err("Soloist rejected a command. Check playback before trying again.".into());
                        }
                        Event::QueueChanged { upcoming } => {
                            if let Some(request) = queue.take() {
                                notify(Update::Queue {
                                    seq: request.seq,
                                    queue: Box::new(Queue {
                                        currently_playing: model.current.as_ref().and_then(Entity::playable),
                                        queue: upcoming.iter().take(QUEUE_LIMIT)
                                            .filter_map(|entry| entry.item.playable()).collect(),
                                    }),
                                });
                            }
                        }
                        Event::Unknown => {}
                        event => {
                            let was_logged_in = model.logged_in;
                            let was_active = model.is_active;
                            let authentication = matches!(event, Event::AuthState { .. });
                            let was_ready = model.ready();
                            model.apply(event)?;
                            if authentication && model.logged_in || !was_active && model.is_active && model.logged_in {
                                send(socket, &WireCommand::GetState).await?;
                                snapshot_deadline = Some(Instant::now());
                            }
                            if was_logged_in && !model.logged_in {
                                controls.clear();
                                queue = None;
                                load = None;
                                // Authentication can be replaced while an
                                // accepted pause awaits dispatch or its reply.
                                // Keep the socket epoch and pause barrier until
                                // the outer loop establishes a new connection.
                                invalidate(admission, notify, model, false);
                                notify(Update::Error("Soloist lost its Spotify login. Pending commands were cancelled.".into()));
                            }
                            if model.auth_seen && !model.logged_in || model.ready() {
                                snapshot_deadline = None;
                            }
                            let ready = model.ready() && priority_pause.is_none() && !admission.pausing.load(Ordering::Acquire);
                            admission.ready.store(ready, Ordering::Release);
                            if ready != was_ready {
                                notify(Update::Ready(ready));
                            }
                            notify(Update::State(model.state.clone()));
                            if model.logged_in && model.is_active && let Some(request) = load.take()
                                && request.load_generation == admission.load_generation.load(Ordering::Acquire) {
                                    notify(Update::Load {
                                        spec: request.spec,
                                        generation: request.session_generation,
                                    });
                            }
                        }
                    }
                }
                request = receiver.recv() => {
                    let Some(Pending { generation, load_generation, session_generation, request, mut ticket }) = request else { return Ok(ConnectionExit::Stop) };
                    if !model.ready() || priority_pause.is_some() || admission.pausing.load(Ordering::Acquire) || generation != admission.generation.load(Ordering::Acquire) {
                        if !admission.pausing.load(Ordering::Acquire) {
                            notify(Update::Error(DISCONNECTED.into()));
                        }
                        continue;
                    }
                    match request {
                        Request::Queue(seq) => {
                            send(socket, &WireCommand::GetQueue { limit: QUEUE_LIMIT as u32 }).await?;
                            queue = Some(QueueRequest { seq, sent: Instant::now(), _ticket: ticket });
                        }
                        Request::Control(PlayerCommand::Load(spec)) => {
                            if load_generation != admission.load_generation.load(Ordering::Acquire) {
                                continue;
                            }
                            if model.is_active {
                                notify(Update::Load { spec, generation: session_generation });
                            } else {
                                send(socket, &WireCommand::Activate).await?;
                                controls.push_back(Control { name: "activate", sent: Instant::now(), _ticket: None });
                                load = Some(LoadRequest { spec, load_generation, session_generation, sent: Instant::now(), _ticket: ticket });
                            }
                        }
                        Request::Control(PlayerCommand::VolumePreview(volume)) => {
                            model.state.volume = volume;
                            notify(Update::State(model.state.clone()));
                        }
                        Request::Control(command) => {
                            for wire in wire_commands(command, model.state.playback)? {
                                // A sign-out racing with a multi-command
                                // mutation invalidates its remaining writes.
                                if admission.pausing.load(Ordering::Acquire) || generation != admission.generation.load(Ordering::Acquire) {
                                    break;
                                }
                                send(socket, &wire).await?;
                                controls.push_back(Control {
                                    name: wire.name(), sent: Instant::now(), _ticket: Some(ticket.split()),
                                });
                            }
                        }
                    }
                }
                _ = tick.tick() => {
                    if priority_pause.is_some_and(|sent| sent.elapsed() >= DEADLINE) {
                        return Err("Soloist did not confirm pause. Check playback before trying again.".into());
                    }
                    if load.as_ref().is_some_and(|request| request.load_generation != admission.load_generation.load(Ordering::Acquire)) {
                        load = None;
                    }
                    if snapshot_deadline.is_some_and(|sent| sent.elapsed() >= DEADLINE) {
                        return Err("Soloist did not provide its current playback state.".into());
                    }
                    if controls.iter().any(|control| control.sent.elapsed() >= DEADLINE) {
                        return Err("Soloist did not confirm a command. Check playback before trying again.".into());
                    }
                    if queue.as_ref().is_some_and(|request| request.sent.elapsed() >= DEADLINE) {
                        return Err("Soloist did not answer the queue request. The connection will refresh.".into());
                    }
                    if load.as_ref().is_some_and(|request| request.sent.elapsed() >= DEADLINE) {
                        load = None;
                        notify(Update::Error("Soloist did not become active. The requested music was not started.".into()));
                    }
                    if received.elapsed() >= Duration::from_secs(45) {
                        return Err("The Soloist runtime stopped responding.".into());
                    }
                    if pinged.elapsed() >= Duration::from_secs(15) {
                        timeout(WRITE_DEADLINE, socket.send(Message::Ping(Vec::new().into())))
                            .await.map_err(|_| "The Soloist runtime socket stopped accepting messages.")?
                            .map_err(|_| "The Soloist runtime socket failed.")?;
                        pinged = Instant::now();
                    }
                }
            }
        }
    }

    #[derive(Serialize)]
    struct CommandFrame<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        #[serde(flatten)]
        command: &'a WireCommand,
    }

    #[derive(Serialize)]
    #[serde(tag = "command", rename_all = "snake_case")]
    enum WireCommand {
        GetState,
        GetQueue { limit: u32 },
        Play,
        Pause,
        SkipNext,
        SkipPrev,
        Seek { position_ms: u32 },
        SetVolume { volume: u8 },
        SetShuffle { enabled: bool },
        SetRepeatContext { enabled: bool },
        SetRepeatTrack { enabled: bool },
        AddToQueue { uri: String },
        Activate,
    }

    impl WireCommand {
        fn name(&self) -> &'static str {
            match self {
                Self::GetState => "get_state",
                Self::GetQueue { .. } => "get_queue",
                Self::Play => "play",
                Self::Pause => "pause",
                Self::SkipNext => "skip_next",
                Self::SkipPrev => "skip_prev",
                Self::Seek { .. } => "seek",
                Self::SetVolume { .. } => "set_volume",
                Self::SetShuffle { .. } => "set_shuffle",
                Self::SetRepeatContext { .. } => "set_repeat_context",
                Self::SetRepeatTrack { .. } => "set_repeat_track",
                Self::AddToQueue { .. } => "add_to_queue",
                Self::Activate => "activate",
            }
        }
    }

    async fn send(socket: &mut Socket, command: &WireCommand) -> Result<(), String> {
        let text = serde_json::to_string(&CommandFrame {
            kind: "command",
            command,
        })
        .map_err(|_| "Could not encode a Soloist command.")?;
        if text.len() > MAX_MESSAGE {
            return Err("The Soloist command is too large.".into());
        }
        timeout(WRITE_DEADLINE, socket.send(Message::Text(text.into())))
            .await
            .map_err(|_| "Sending a Soloist command timed out. It was not retried.")?
            .map_err(|_| "Sending a Soloist command failed. It was not retried.".to_string())
    }

    fn wire_commands(
        command: PlayerCommand,
        playback: Playback,
    ) -> Result<Vec<WireCommand>, String> {
        Ok(match command {
            PlayerCommand::Toggle => {
                vec![
                    if matches!(playback, Playback::Playing | Playback::Loading) {
                        WireCommand::Pause
                    } else {
                        WireCommand::Play
                    },
                ]
            }
            PlayerCommand::Next => vec![WireCommand::SkipNext],
            PlayerCommand::Previous => vec![WireCommand::SkipPrev],
            PlayerCommand::Seek(position_ms) => vec![WireCommand::Seek { position_ms }],
            PlayerCommand::Volume(volume) => vec![WireCommand::SetVolume {
                volume: ((u32::from(volume) * 100 + u32::from(u16::MAX) / 2) / u32::from(u16::MAX))
                    as u8,
            }],
            PlayerCommand::Shuffle(enabled) => vec![WireCommand::SetShuffle { enabled }],
            PlayerCommand::Repeat(RepeatMode::Off) => vec![
                WireCommand::SetRepeatTrack { enabled: false },
                WireCommand::SetRepeatContext { enabled: false },
            ],
            PlayerCommand::Repeat(RepeatMode::Context) => vec![
                WireCommand::SetRepeatTrack { enabled: false },
                WireCommand::SetRepeatContext { enabled: true },
            ],
            PlayerCommand::Repeat(RepeatMode::Track) => vec![
                WireCommand::SetRepeatContext { enabled: false },
                WireCommand::SetRepeatTrack { enabled: true },
            ],
            PlayerCommand::AddToQueue(uri) => vec![WireCommand::AddToQueue { uri }],
            PlayerCommand::Transfer => vec![WireCommand::Activate],
            PlayerCommand::ClearQueue => {
                return Err("Soloist cannot clear the manual queue.".into());
            }
            PlayerCommand::Load(_) | PlayerCommand::VolumePreview(_) => {
                return Err("This Soloist command requires local handling.".into());
            }
        })
    }

    #[derive(Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum Event {
        AuthState {
            logged_in: bool,
            is_active: bool,
        },
        PlaybackState {
            status: Status,
            #[serde(default)]
            item: Option<Entity>,
            #[serde(default)]
            context: Option<Entity>,
            #[serde(default)]
            position: Option<Position>,
            volume: u8,
            is_active: bool,
            #[serde(default)]
            options: Options,
        },
        TrackChanged {
            #[serde(default)]
            item: Option<Entity>,
        },
        PlaybackChanged {
            status: Status,
        },
        VolumeChanged {
            volume: u8,
        },
        DeviceChanged {
            is_active: bool,
        },
        ContextChanged {
            #[serde(default)]
            context: Option<Entity>,
        },
        OptionsChanged {
            options: Options,
        },
        PositionSync {
            position: Position,
        },
        QueueChanged {
            upcoming: Vec<QueueEntry>,
        },
        CommandResult {
            command: String,
        },
        Error,
        #[serde(other)]
        Unknown,
    }

    #[derive(Clone, Copy, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum Status {
        Idle,
        Playing,
        Paused,
        Buffering,
    }

    #[derive(Clone, Copy, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum Repeat {
        Off,
        Context,
        Track,
    }

    #[derive(Default, Deserialize)]
    struct Options {
        shuffle: Option<bool>,
        repeat: Option<Repeat>,
    }

    #[derive(Clone, Copy, Deserialize)]
    struct Position {
        position_ms: u32,
        timestamp_ms: u64,
        speed: f64,
    }

    #[derive(Default)]
    struct Model {
        state: LocalState,
        logged_in: bool,
        auth_seen: bool,
        seen_state: bool,
        is_active: bool,
        speed: f64,
        status: Option<Status>,
        anchor: Option<Position>,
        current: Option<Entity>,
        context: Option<Entity>,
    }

    impl Model {
        fn ready(&self) -> bool {
            self.logged_in && self.seen_state
        }

        fn disconnect(&mut self) {
            self.state.position_ms = self.state.position_now();
            self.state.position_at = None;
            self.state.playback = Playback::Stopped;
            self.state.connected = false;
            self.state.loading = false;
            self.state.active_client.clear();
            self.logged_in = false;
            self.auth_seen = false;
            self.seen_state = false;
            self.is_active = false;
            self.speed = 0.0;
            self.status = None;
            self.anchor = None;
            self.current = None;
            self.context = None;
        }

        fn apply(&mut self, event: Event) -> Result<(), String> {
            // A late snapshot from a replaced login must not become the next
            // session's current song. Authentication starts a fresh snapshot.
            if !self.logged_in && !matches!(event, Event::AuthState { .. }) {
                return Ok(());
            }
            match event {
                Event::AuthState {
                    logged_in,
                    is_active,
                } => {
                    self.auth_seen = true;
                    self.logged_in = logged_in;
                    self.is_active = is_active && logged_in;
                    self.seen_state = false;
                    if !logged_in {
                        self.disconnect();
                        self.state.track = None;
                        self.state.track_sequence = self.state.track_sequence.wrapping_add(1);
                        self.auth_seen = true;
                    }
                }
                Event::PlaybackState {
                    status,
                    item,
                    context,
                    position,
                    volume,
                    is_active,
                    options,
                } => {
                    self.seen_state = true;
                    self.is_active = is_active;
                    self.set_item(item, false);
                    self.context = context;
                    self.set_status(status);
                    self.set_volume(volume)?;
                    self.set_options(options);
                    if let Some(position) = position {
                        self.set_position(position)?;
                    } else {
                        self.state.position_at = None;
                        self.speed = 0.0;
                    }
                }
                Event::TrackChanged { item } => self.set_item(item, true),
                Event::PlaybackChanged { status } => self.set_status(status),
                Event::VolumeChanged { volume } => self.set_volume(volume)?,
                Event::DeviceChanged { is_active } => {
                    self.is_active = is_active;
                    if is_active && let Some(status) = self.status {
                        self.set_status(status);
                        if let Some(anchor) = self.anchor {
                            self.set_position(anchor)?;
                        }
                    }
                }
                Event::ContextChanged { context } => self.context = context,
                Event::OptionsChanged { options } => self.set_options(options),
                Event::PositionSync { position } => self.set_position(position)?,
                _ => {}
            }
            self.state.connected = self.ready();
            if !self.logged_in || !self.is_active {
                self.state.position_ms = self.state.position_now();
                self.state.position_at = None;
                self.state.playback = Playback::Stopped;
                self.state.loading = false;
            }
            Ok(())
        }

        fn set_item(&mut self, item: Option<Entity>, replay: bool) {
            let track = item.as_ref().and_then(Entity::local_track);
            if replay || track != self.state.track {
                self.state.track_sequence = self.state.track_sequence.wrapping_add(1);
                self.state.position_ms = 0;
                self.state.position_at = None;
                self.speed = 0.0;
                self.anchor = None;
            }
            self.state.track = track;
            self.current = item;
        }

        fn set_status(&mut self, status: Status) {
            self.status = Some(status);
            self.state.position_ms = self.state.position_now();
            self.state.position_at = None;
            self.state.playback = match status {
                Status::Idle => Playback::Stopped,
                Status::Playing => Playback::Playing,
                Status::Paused => Playback::Paused,
                Status::Buffering => Playback::Loading,
            };
            self.state.loading = matches!(status, Status::Buffering);
            if matches!(status, Status::Playing) && (self.speed - 1.0).abs() < f64::EPSILON {
                self.state.position_at = Some(Instant::now());
            }
        }

        fn set_volume(&mut self, volume: u8) -> Result<(), String> {
            if volume > 100 {
                return Err("Soloist reported an invalid volume.".into());
            }
            self.state.volume = (u32::from(volume) * u32::from(u16::MAX) / 100) as u16;
            Ok(())
        }

        fn set_options(&mut self, options: Options) {
            if let Some(shuffle) = options.shuffle {
                self.state.shuffle = shuffle;
            }
            if let Some(repeat) = options.repeat {
                self.state.repeat = match repeat {
                    Repeat::Off => RepeatMode::Off,
                    Repeat::Context => RepeatMode::Context,
                    Repeat::Track => RepeatMode::Track,
                };
            }
        }

        fn set_position(&mut self, position: Position) -> Result<(), String> {
            if !position.speed.is_finite() || !(0.0..=4.0).contains(&position.speed) {
                return Err("Soloist reported an unsupported position speed.".into());
            }
            self.speed = position.speed;
            self.anchor = Some(position);
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let age = now.saturating_sub(u128::from(position.timestamp_ms));
            let advancing = self.state.playback == Playback::Playing && self.is_active;
            let elapsed = if advancing {
                (age as f64 * position.speed) as u32
            } else {
                0
            };
            let limit = self.state.track.as_ref().map_or(u32::MAX, |track| {
                if track.duration_ms == 0 {
                    u32::MAX
                } else {
                    track.duration_ms
                }
            });
            self.state.position_ms = position.position_ms.saturating_add(elapsed).min(limit);
            // LocalState's interpolator models ordinary music playback at 1x.
            // Other documented speeds retain their anchor until the next sync.
            self.state.position_at =
                (advancing && (position.speed - 1.0).abs() < f64::EPSILON).then(Instant::now);
            self.state.seek_sequence = self.state.seek_sequence.wrapping_add(1);
            Ok(())
        }
    }

    #[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    enum EntityKind {
        Track,
        Episode,
        Artist,
        Album,
        Playlist,
        Show,
        Ad,
        #[default]
        #[serde(other)]
        Unknown,
    }

    #[derive(Default, Deserialize)]
    struct Entity {
        #[serde(default)]
        uri: String,
        #[serde(default)]
        entity_type: EntityKind,
        #[serde(default)]
        decorations: Decorations,
    }

    #[derive(Default, Deserialize)]
    struct Decorations {
        #[serde(default)]
        identity: Identity,
        #[serde(default)]
        visual_identity: VisualIdentity,
        parent: Option<Related>,
        #[serde(default)]
        creators: Vec<Related>,
        #[serde(default)]
        playback: DecorationPlayback,
    }

    #[derive(Default, Deserialize)]
    struct Identity {
        #[serde(default)]
        name: String,
    }

    #[derive(Default, Deserialize)]
    struct VisualIdentity {
        #[serde(default)]
        cover: Vec<Cover>,
    }

    #[derive(Deserialize)]
    struct Cover {
        url: String,
        #[serde(default)]
        size: CoverSize,
    }

    #[derive(Clone, Copy, Default, Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum CoverSize {
        Small,
        #[default]
        Default,
        Large,
        Xlarge,
        #[serde(other)]
        Unknown,
    }

    #[derive(Deserialize)]
    struct Related {
        entity: Box<Entity>,
    }

    #[derive(Default, Deserialize)]
    struct DecorationPlayback {
        #[serde(default)]
        duration_ms: u32,
        #[serde(default)]
        content_ratings: Vec<String>,
    }

    #[derive(Deserialize)]
    struct QueueEntry {
        item: Entity,
    }

    fn spotify_id<'a>(uri: &'a str, kind: &str) -> Option<&'a str> {
        let tail = uri
            .strip_prefix("spotify:")?
            .strip_prefix(kind)?
            .strip_prefix(':')?;
        (!tail.is_empty()
            && tail.len() <= 128
            && tail.bytes().all(|byte| byte.is_ascii_alphanumeric()))
        .then_some(tail)
    }

    impl Entity {
        fn name(&self) -> String {
            if self.decorations.identity.name.is_empty() {
                self.uri.clone()
            } else {
                self.decorations.identity.name.clone()
            }
        }

        fn images(&self) -> Vec<Image> {
            self.decorations
                .visual_identity
                .cover
                .iter()
                .take(8)
                .filter_map(|cover| {
                    let url = reqwest::Url::parse(&cover.url).ok()?;
                    let host = url.host_str()?;
                    if url.scheme() != "https"
                        || !url.username().is_empty()
                        || url.password().is_some()
                        || !(host == "i.scdn.co"
                            || host == "mosaic.scdn.co"
                            || host.ends_with(".spotifycdn.com"))
                    {
                        return None;
                    }
                    let width = match cover.size {
                        CoverSize::Small => Some(64),
                        CoverSize::Default => Some(300),
                        CoverSize::Large => Some(640),
                        CoverSize::Xlarge => Some(1024),
                        CoverSize::Unknown => None,
                    };
                    Some(Image {
                        url: cover.url.clone(),
                        width,
                        height: width,
                    })
                })
                .collect()
        }

        fn artists(&self) -> Vec<ArtistRef> {
            self.decorations
                .creators
                .iter()
                .take(32)
                .filter_map(|related| {
                    let artist = &related.entity;
                    if artist.entity_type != EntityKind::Artist {
                        return None;
                    }
                    let id = spotify_id(&artist.uri, "artist");
                    if artist.decorations.identity.name.is_empty() && id.is_none() {
                        return None;
                    }
                    Some(ArtistRef {
                        id: id.map(str::to_string),
                        name: artist.name(),
                        uri: id.map(|_| artist.uri.clone()),
                    })
                })
                .collect()
        }

        fn parent(&self, kind: EntityKind) -> Option<&Entity> {
            self.decorations
                .parent
                .as_ref()
                .map(|related| related.entity.as_ref())
                .filter(|parent| parent.entity_type == kind)
        }

        fn playable(&self) -> Option<PlayableItem> {
            let duration_ms = self.decorations.playback.duration_ms;
            let explicit = self
                .decorations
                .playback
                .content_ratings
                .iter()
                .any(|rating| rating == "explicit");
            match self.entity_type {
                EntityKind::Track => {
                    let id = spotify_id(&self.uri, "track")?;
                    let own_images = self.images();
                    let parent = self.parent(EntityKind::Album);
                    let album = parent
                        .map(|parent| {
                            let mut images = parent.images();
                            if images.is_empty() {
                                images.clone_from(&own_images);
                            }
                            Album {
                                id: spotify_id(&parent.uri, "album").unwrap_or_default().into(),
                                uri: parent.uri.clone(),
                                name: parent.name(),
                                images,
                                ..Album::default()
                            }
                        })
                        .or_else(|| {
                            (!own_images.is_empty()).then(|| Album {
                                images: own_images,
                                ..Album::default()
                            })
                        });
                    Some(PlayableItem::Track(Track {
                        id: Some(id.into()),
                        uri: self.uri.clone(),
                        name: self.name(),
                        duration_ms,
                        explicit,
                        artists: self.artists(),
                        album,
                        ..Track::default()
                    }))
                }
                EntityKind::Episode => {
                    let id = spotify_id(&self.uri, "episode")?;
                    let show = self.parent(EntityKind::Show).map(|parent| Show {
                        id: spotify_id(&parent.uri, "show").unwrap_or_default().into(),
                        uri: parent.uri.clone(),
                        name: parent.name(),
                        images: parent.images(),
                        ..Show::default()
                    });
                    let mut images = self.images();
                    if images.is_empty()
                        && let Some(show) = &show
                    {
                        images.clone_from(&show.images);
                    }
                    Some(PlayableItem::Episode(Episode {
                        id: id.into(),
                        uri: self.uri.clone(),
                        name: self.name(),
                        duration_ms,
                        explicit,
                        images,
                        show,
                        ..Episode::default()
                    }))
                }
                _ => None,
            }
        }

        fn local_track(&self) -> Option<LocalTrack> {
            let item = self.playable()?;
            Some(match item {
                PlayableItem::Track(track) => LocalTrack {
                    uri: track.uri.clone(),
                    title: track.name.clone(),
                    artists: track.artists.clone(),
                    album: track
                        .album
                        .as_ref()
                        .map_or_else(String::new, |album| album.name.clone()),
                    art_url: track.image(640).map(str::to_string),
                    art_small_url: track.image(64).map(str::to_string),
                    duration_ms: track.duration_ms,
                    is_episode: false,
                },
                PlayableItem::Episode(episode) => LocalTrack {
                    uri: episode.uri.clone(),
                    title: episode.name.clone(),
                    artists: self.artists(),
                    album: episode
                        .show
                        .as_ref()
                        .map_or_else(String::new, |show| show.name.clone()),
                    art_url: crate::api::models::pick_image(&episode.images, 640)
                        .map(str::to_string),
                    art_small_url: crate::api::models::pick_image(&episode.images, 64)
                        .map(str::to_string),
                    duration_ms: episode.duration_ms,
                    is_episode: true,
                },
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use serde_json::{Value, json};
        use std::os::unix::fs::{PermissionsExt, symlink};
        use tokio::net::UnixListener;

        struct PrivateDirectory(PathBuf);

        impl PrivateDirectory {
            fn new() -> Self {
                let path = std::env::temp_dir().join(format!(
                    "spotifast-soloist-{}-{}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
                std::fs::create_dir(&path).unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
                Self(path)
            }

            fn socket(&self) -> PathBuf {
                // Keep macOS sockaddr_un's short path limit in mind.
                self.0.join("s")
            }
        }

        impl Drop for PrivateDirectory {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        fn entity() -> Value {
            json!({
                "uri": "spotify:track:abc123", "entity_type": "track",
                "decorations": {
                    "identity": { "name": "Music" },
                    "visual_identity": { "cover": [
                        { "url": "https://i.scdn.co/image/small", "size": "small" },
                        { "url": "https://i.scdn.co/image/large", "size": "large" },
                        { "url": "http://127.0.0.1/private", "size": "xlarge" }
                    ] },
                    "parent": { "entity": {
                        "uri": "spotify:album:album123", "entity_type": "album",
                        "decorations": { "identity": { "name": "Album" } }
                    } },
                    "creators": [{ "entity": {
                        "uri": "spotify:artist:artist123", "entity_type": "artist",
                        "decorations": { "identity": { "name": "Artist" } }
                    } }],
                    "playback": { "duration_ms": 210000, "content_ratings": ["explicit"] }
                },
                "lossless": true, "bitrate": 1411
            })
        }

        fn snapshot(active: bool) -> Value {
            json!({
                "type": "playback_state", "status": "playing", "item": entity(),
                "is_active": active, "volume": 65,
                "position": { "position_ms": 45000, "timestamp_ms": now_ms(), "speed": 1.0 },
                "options": { "shuffle": true, "repeat": "context" }
            })
        }

        fn now_ms() -> u64 {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
        }

        fn event(value: Value) -> Event {
            serde_json::from_value(value).unwrap()
        }

        #[test]
        fn socket_requires_private_directory_owned_socket_and_no_symlinks() {
            let directory = PrivateDirectory::new();
            let path = directory.socket();
            let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert!(validate_socket(&path, current_uid()).is_ok());
            assert!(validate_socket(&path, current_uid().wrapping_add(1)).is_err());
            assert!(validate_socket(Path::new("relative.sock"), current_uid()).is_err());

            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
            assert!(validate_socket(&path, current_uid()).is_err());
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            std::fs::set_permissions(&directory.0, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(validate_socket(&path, current_uid()).is_err());
            std::fs::set_permissions(&directory.0, std::fs::Permissions::from_mode(0o700)).unwrap();

            let link = directory.0.join("link");
            symlink(&path, &link).unwrap();
            assert!(validate_socket(&link, current_uid()).is_err());
            drop(listener);
            std::fs::remove_file(&path).unwrap();
            std::fs::write(&path, b"not a socket").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert!(validate_socket(&path, current_uid()).is_err());
        }

        #[test]
        fn documented_entity_envelope_maps_music_and_artwork() {
            let entity: Entity = serde_json::from_value(entity()).unwrap();
            let local = entity.local_track().unwrap();
            assert_eq!(local.uri, "spotify:track:abc123");
            assert_eq!(local.title, "Music");
            assert_eq!(local.album, "Album");
            assert_eq!(local.artists[0].name, "Artist");
            assert_eq!(
                local.artists[0].uri.as_deref(),
                Some("spotify:artist:artist123")
            );
            assert_eq!(local.duration_ms, 210000);
            assert_eq!(
                local.art_url.as_deref(),
                Some("https://i.scdn.co/image/large")
            );
            assert_eq!(
                local.art_small_url.as_deref(),
                Some("https://i.scdn.co/image/small")
            );
            assert!(!local.is_episode);
            let Some(PlayableItem::Track(track)) = entity.playable() else {
                panic!("track")
            };
            assert!(track.explicit);
            assert_eq!(track.album.unwrap().images.len(), 2);
        }

        #[test]
        fn missing_metadata_is_safe_and_unknown_entities_cannot_spoof_tracks() {
            let minimal: Entity = serde_json::from_value(json!({
                "uri": "spotify:track:abc123", "entity_type": "track"
            }))
            .unwrap();
            let track = minimal.local_track().unwrap();
            assert_eq!(track.title, "spotify:track:abc123");
            assert!(track.artists.is_empty());
            assert!(track.art_url.is_none());
            assert_eq!(track.duration_ms, 0);
            for value in [
                json!({ "uri": "spotify:track:abc123", "entity_type": "unknown" }),
                json!({ "uri": "spotify:episode:abc123", "entity_type": "track" }),
                json!({ "uri": "file:///private/data", "entity_type": "track" }),
                json!({ "entity_type": "track" }),
            ] {
                let entity: Entity = serde_json::from_value(value).unwrap();
                assert!(entity.playable().is_none());
            }
        }

        #[test]
        fn authentication_loss_clears_track_and_requires_a_fresh_snapshot() {
            let mut model = Model::default();
            model
                .apply(event(
                    json!({ "type": "auth_state", "logged_in": true, "is_active": true }),
                ))
                .unwrap();
            assert!(!model.ready());
            model.apply(event(snapshot(true))).unwrap();
            assert!(model.ready() && model.state.connected);
            assert_eq!(model.state.playback, Playback::Playing);
            assert!(model.state.shuffle);
            assert_eq!(model.state.repeat, RepeatMode::Context);
            model
                .apply(event(
                    json!({ "type": "auth_state", "logged_in": false, "is_active": false }),
                ))
                .unwrap();
            assert!(model.state.track.is_none());
            assert!(!model.ready() && !model.state.connected);
            assert!(model.state.position_at.is_none());
            assert_eq!(model.state.playback, Playback::Stopped);
            // An old session's delayed snapshot is ignored after logout.
            model.apply(event(snapshot(true))).unwrap();
            model
                .apply(event(
                    json!({ "type": "auth_state", "logged_in": true, "is_active": true }),
                ))
                .unwrap();
            assert!(!model.ready());
            model.apply(event(snapshot(true))).unwrap();
            assert!(model.ready());
        }

        #[test]
        fn position_anchors_and_same_uri_replays_update_their_sequences() {
            let mut model = Model::default();
            model
                .apply(event(
                    json!({ "type": "auth_state", "logged_in": true, "is_active": true }),
                ))
                .unwrap();
            model.apply(event(snapshot(true))).unwrap();
            model
                .apply(event(json!({ "type": "position_sync", "position": {
                "position_ms": 1000, "timestamp_ms": now_ms().saturating_sub(1000), "speed": 1.0
            } })))
                .unwrap();
            assert!(model.state.position_ms >= 2000 && model.state.position_ms < 3000);
            assert!(model.state.position_at.is_some());
            let sequence = model.state.track_sequence;
            model
                .apply(event(json!({ "type": "track_changed", "item": entity() })))
                .unwrap();
            assert!(model.state.track_sequence > sequence);
            assert_eq!(model.state.position_ms, 0);
            assert!(model.state.position_at.is_none());
            model
                .apply(event(json!({ "type": "position_sync", "position": {
                "position_ms": 3000, "timestamp_ms": now_ms(), "speed": 0.0
            } })))
                .unwrap();
            assert_eq!(model.state.position_ms, 3000);
            assert!(model.state.position_at.is_none());
        }

        #[test]
        fn command_mapping_uses_documented_fields_and_explicit_limits() {
            let repeat =
                wire_commands(PlayerCommand::Repeat(RepeatMode::Track), Playback::Paused).unwrap();
            let frames: Vec<Value> = repeat
                .iter()
                .map(|command| {
                    serde_json::to_value(CommandFrame {
                        kind: "command",
                        command,
                    })
                    .unwrap()
                })
                .collect();
            assert_eq!(
                frames,
                vec![
                    json!({ "type": "command", "command": "set_repeat_context", "enabled": false }),
                    json!({ "type": "command", "command": "set_repeat_track", "enabled": true }),
                ]
            );
            assert!(matches!(
                wire_commands(PlayerCommand::Toggle, Playback::Paused).unwrap()[0],
                WireCommand::Play
            ));
            assert!(matches!(
                wire_commands(PlayerCommand::Toggle, Playback::Playing).unwrap()[0],
                WireCommand::Pause
            ));
            assert!(matches!(
                wire_commands(PlayerCommand::Volume(u16::MAX), Playback::Paused).unwrap()[0],
                WireCommand::SetVolume { volume: 100 }
            ));
            assert!(validate_command(&PlayerCommand::ClearQueue).is_err());
            assert!(
                validate_command(&PlayerCommand::AddToQueue("spotify:episode:abc123".into()))
                    .is_err()
            );
        }

        #[test]
        fn outstanding_commands_are_bounded_until_their_acknowledgements() {
            let admission = Arc::new(Admission::default());
            let mut tickets: Vec<_> = (0..LIMIT)
                .map(|_| Ticket::reserve(&admission, 1).unwrap())
                .collect();
            assert!(Ticket::reserve(&admission, 1).is_err());
            tickets.pop();
            assert!(Ticket::reserve(&admission, 2).is_err());
            tickets.pop();
            let mut repeat = Ticket::reserve(&admission, 2).unwrap();
            let first = repeat.split();
            let second = repeat.split();
            drop(repeat);
            assert_eq!(admission.outstanding.load(Ordering::Acquire), LIMIT);
            drop((first, second, tickets));
            assert_eq!(admission.outstanding.load(Ordering::Acquire), 0);
        }

        #[test]
        fn pause_cancels_pending_load_admission_and_never_toggles() {
            let (sender, mut receiver) = mpsc::channel(LIMIT);
            let (pause_sender, mut pauses) = watch::channel(0);
            let admission = Arc::new(Admission::default());
            admission.ready.store(true, Ordering::Release);
            let handle = Handle {
                sender,
                pause_sender,
                admission: Arc::clone(&admission),
                task: None,
            };
            handle
                .command(PlayerCommand::Load(LoadSpec {
                    context_uri: Some("spotify:album:album123".into()),
                    ..LoadSpec::default()
                }))
                .unwrap();
            let load = receiver.try_recv().unwrap();
            handle.pause().unwrap();
            assert!(load.load_generation < admission.load_generation.load(Ordering::Acquire));
            assert!(load.generation < admission.generation.load(Ordering::Acquire));
            assert!(pauses.has_changed().unwrap());
            assert_eq!(
                *pauses.borrow_and_update(),
                admission.connection_generation.load(Ordering::Acquire)
            );
            assert!(receiver.try_recv().is_err());
            drop(load);
            assert_eq!(admission.outstanding.load(Ordering::Acquire), 0);
            assert!(!admission.load.load(Ordering::Acquire));
        }

        #[test]
        fn saturated_mutation_queue_cannot_reject_priority_pause() {
            let (sender, mut receiver) = mpsc::channel(LIMIT);
            let (pause_sender, mut pauses) = watch::channel(0);
            let admission = Arc::new(Admission::default());
            admission.ready.store(true, Ordering::Release);
            let handle = Handle {
                sender,
                pause_sender,
                admission: Arc::clone(&admission),
                task: None,
            };
            for _ in 0..LIMIT {
                handle.command(PlayerCommand::Toggle).unwrap();
            }
            assert_eq!(admission.outstanding.load(Ordering::Acquire), LIMIT);
            assert!(handle.command(PlayerCommand::Transfer).is_err());
            // Pause must be accepted independently of both ordinary channel
            // capacity and outstanding command acknowledgement tickets.
            handle.pause().unwrap();
            let paused_generation = admission.generation.load(Ordering::Acquire);
            assert_eq!(
                *pauses.borrow_and_update(),
                admission.connection_generation.load(Ordering::Acquire)
            );
            for _ in 0..LIMIT {
                let pending = receiver.try_recv().unwrap();
                assert!(pending.generation < paused_generation);
            }
            assert!(receiver.try_recv().is_err());
            assert_eq!(admission.outstanding.load(Ordering::Acquire), 0);
            // The signal remains bounded and always represents the latest
            // cancellation epoch, even when sign-out is requested repeatedly.
            handle.pause().unwrap();
            handle.pause().unwrap();
            assert!(admission.generation.load(Ordering::Acquire) > paused_generation);
            assert!(pauses.has_changed().unwrap());
            assert_eq!(
                *pauses.borrow_and_update(),
                admission.connection_generation.load(Ordering::Acquire)
            );
        }

        #[test]
        fn accepted_pause_keeps_its_socket_epoch_through_authentication_change() {
            let (sender, _receiver) = mpsc::channel(LIMIT);
            let (pause_sender, mut pauses) = watch::channel(0);
            let admission = Arc::new(Admission::default());
            admission.ready.store(true, Ordering::Release);
            admission.connection_generation.store(7, Ordering::Release);
            let handle = Handle {
                sender,
                pause_sender,
                admission: Arc::clone(&admission),
                task: None,
            };
            let notify: Notify = Arc::new(|_| {});
            let mut model = Model::default();
            handle.pause().unwrap();
            let mutation_generation = admission.generation.load(Ordering::Acquire);
            // Authentication may expire before the transport observes the
            // accepted watch signal. It cancels mutations without making
            // that signal stale or releasing its admission barrier.
            invalidate(&admission, &notify, &mut model, false);
            assert!(admission.generation.load(Ordering::Acquire) > mutation_generation);
            assert!(admission.pausing.load(Ordering::Acquire));
            assert_eq!(admission.connection_generation.load(Ordering::Acquire), 7);
            assert_eq!(*pauses.borrow_and_update(), 7);
            // A real socket replacement releases the barrier and changes
            // the epoch. A delayed old signal is now explicitly stale.
            invalidate(&admission, &notify, &mut model, true);
            assert!(!admission.pausing.load(Ordering::Acquire));
            assert!(admission.connection_generation.load(Ordering::Acquire) > 7);
            assert_eq!(*pauses.borrow_and_update(), 7);
        }

        async fn server_send(socket: &mut Socket, value: Value) {
            socket
                .send(Message::Text(value.to_string().into()))
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn priority_pause_precedes_saturated_mutations_and_cancels_active_load() {
            let directory = PrivateDirectory::new();
            let path = directory.socket();
            let listener = UnixListener::bind(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            let (commands, mut received) = mpsc::unbounded_channel();
            let (cycled, auth_replaced) = tokio::sync::oneshot::channel();
            let (release, released) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let mut cycled = Some(cycled);
                let mut released = Some(released);
                loop {
                    let (stream, _) = listener.accept().await.unwrap();
                    let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                    server_send(
                        &mut socket,
                        json!({ "type": "auth_state", "logged_in": true, "is_active": true }),
                    )
                    .await;
                    while let Some(Ok(Message::Text(text))) = socket.next().await {
                        let value: Value = serde_json::from_str(text.as_ref()).unwrap();
                        let command = value["command"].as_str().unwrap();
                        if command == "get_state" {
                            server_send(&mut socket, snapshot(true)).await;
                        } else {
                            commands.send(command.to_owned()).unwrap();
                            if command == "pause"
                                && let Some(cycled) = cycled.take()
                            {
                                // Replacing authentication before the pause
                                // acknowledgement must not release the old
                                // connection's mutation-admission barrier.
                                server_send(&mut socket, json!({ "type": "auth_state", "logged_in": false, "is_active": false })).await;
                                server_send(&mut socket, json!({ "type": "auth_state", "logged_in": true, "is_active": true })).await;
                                server_send(&mut socket, snapshot(true)).await;
                                cycled.send(()).unwrap();
                                released.take().unwrap().await.unwrap();
                            }
                            server_send(
                                &mut socket,
                                json!({ "type": "command_result", "command": command }),
                            )
                            .await;
                            if command == "pause" {
                                break;
                            }
                        }
                    }
                }
            });
            let (updates, mut update_receiver) = mpsc::unbounded_channel();
            let handle = Handle::start(
                path,
                Arc::new(move |update| {
                    let _ = updates.send(update);
                }),
            );
            timeout(Duration::from_secs(2), async {
                loop {
                    match update_receiver.recv().await.unwrap() {
                        Update::Ready(true) => break,
                        Update::Error(error) => panic!("{error}"),
                        _ => {}
                    }
                }
            })
            .await
            .unwrap();
            while update_receiver.try_recv().is_ok() {}
            // This current-thread fixture does not yield while filling the
            // ordinary queue, so the cancellation races with a full batch.
            handle
                .command_for_session(
                    PlayerCommand::Load(LoadSpec {
                        context_uri: Some("spotify:album:album123".into()),
                        ..LoadSpec::default()
                    }),
                    41,
                )
                .unwrap();
            for index in 1..LIMIT {
                let command = match index % 7 {
                    0 => PlayerCommand::Toggle,
                    1 => PlayerCommand::Transfer,
                    2 => PlayerCommand::Seek(1000),
                    3 => PlayerCommand::Volume(u16::MAX),
                    4 => PlayerCommand::Shuffle(true),
                    5 => PlayerCommand::Next,
                    _ => PlayerCommand::Previous,
                };
                handle.command(command).unwrap();
            }
            assert_eq!(handle.sender.capacity(), 0);
            let old_connection = handle
                .admission
                .connection_generation
                .load(Ordering::Acquire);
            handle.pause().unwrap();
            assert!(handle.command(PlayerCommand::Transfer).is_err());
            assert_eq!(
                timeout(Duration::from_secs(2), received.recv())
                    .await
                    .unwrap()
                    .as_deref(),
                Some("pause")
            );
            timeout(Duration::from_secs(2), auth_replaced)
                .await
                .unwrap()
                .unwrap();
            timeout(Duration::from_secs(2), async {
                loop {
                    match update_receiver.recv().await.unwrap() {
                        Update::Ready(true) => panic!("auth replacement cannot release pause"),
                        Update::Load { .. } => panic!("pause must cancel the admitted load"),
                        Update::State(state) if state.connected => break,
                        _ => {}
                    }
                }
            })
            .await
            .unwrap();
            assert!(!handle.admission.ready.load(Ordering::Acquire));
            assert!(handle.admission.pausing.load(Ordering::Acquire));
            assert!(handle.command(PlayerCommand::Transfer).is_err());
            release.send(()).unwrap();
            timeout(Duration::from_secs(2), async {
                while handle.admission.outstanding.load(Ordering::Acquire) != 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            timeout(Duration::from_secs(2), async {
                loop {
                    match update_receiver.recv().await.unwrap() {
                        Update::Load { .. } => panic!("pause must cancel the admitted load"),
                        Update::Ready(true) => break,
                        Update::Error(error) => panic!("{error}"),
                        _ => {}
                    }
                }
            })
            .await
            .unwrap();
            assert!(received.try_recv().is_err());
            assert!(
                handle
                    .admission
                    .connection_generation
                    .load(Ordering::Acquire)
                    > old_connection
            );
            // A pause thread delayed until after reconnect retains the old
            // socket epoch. Its signal must not affect the fresh connection.
            handle.pause_sender.send(old_connection).unwrap();
            // New commands use a fresh connection and state snapshot, so old
            // named acknowledgements cannot confirm these new mutations.
            handle.command(PlayerCommand::Volume(0)).unwrap();
            assert_eq!(
                timeout(Duration::from_secs(2), received.recv())
                    .await
                    .unwrap()
                    .as_deref(),
                Some("set_volume")
            );
            drop(handle);
            server.abort();
        }

        async fn receive_load(receiver: &mut mpsc::UnboundedReceiver<Update>) -> (LoadSpec, u64) {
            loop {
                if let Some(Update::Load { spec, generation }) = receiver.recv().await {
                    return (spec, generation);
                }
            }
        }

        #[tokio::test]
        async fn load_keeps_admitted_account_token_and_context_offsets_after_activation() {
            activation_load(false).await;
        }

        #[tokio::test]
        async fn authentication_loss_cancels_activation_load_without_replaying_it() {
            activation_load(true).await;
        }

        async fn activation_load(lose_auth: bool) {
            let directory = PrivateDirectory::new();
            let path = directory.socket();
            let listener = UnixListener::bind(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            let (activated, activation) = tokio::sync::oneshot::channel();
            let (release, released) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                server_send(
                    &mut socket,
                    json!({ "type": "auth_state", "logged_in": true, "is_active": false }),
                )
                .await;
                while let Some(Ok(Message::Text(text))) = socket.next().await {
                    let value: Value = serde_json::from_str(text.as_ref()).unwrap();
                    match value["command"].as_str().unwrap() {
                        "get_state" => server_send(&mut socket, snapshot(false)).await,
                        "activate" => {
                            server_send(
                                &mut socket,
                                json!({ "type": "command_result", "command": "activate" }),
                            )
                            .await;
                            let _ = activated.send(());
                            let _ = released.await;
                            if lose_auth {
                                server_send(&mut socket, json!({ "type": "auth_state", "logged_in": false, "is_active": false })).await;
                            }
                            server_send(
                                &mut socket,
                                json!({ "type": "device_changed", "is_active": true }),
                            )
                            .await;
                            while socket.next().await.is_some() {}
                            break;
                        }
                        other => panic!("unexpected command {other}"),
                    }
                }
            });
            let (updates, mut receiver) = mpsc::unbounded_channel();
            let handle = Handle::start(
                path,
                Arc::new(move |update| {
                    let _ = updates.send(update);
                }),
            );
            timeout(Duration::from_secs(2), async {
                loop {
                    match receiver.recv().await.unwrap() {
                        Update::Ready(true) => break,
                        Update::Error(error) => panic!("{error}"),
                        _ => {}
                    }
                }
            })
            .await
            .unwrap();
            let spec = LoadSpec {
                context_uri: Some("spotify:album:album123".into()),
                uris: vec![
                    "spotify:track:abc123".into(),
                    "spotify:track:other123".into(),
                ],
                offset_uri: Some("spotify:track:other123".into()),
                offset_index: Some(1),
                position_ms: 12345,
                play: true,
                shuffle: Some(false),
                repeat: Some(RepeatMode::Track),
                autoplay: true,
            };
            let account_generation = AtomicU64::new(41);
            handle
                .command_for_session(
                    PlayerCommand::Load(spec.clone()),
                    account_generation.load(Ordering::Acquire),
                )
                .unwrap();
            timeout(Duration::from_secs(2), activation)
                .await
                .unwrap()
                .unwrap();
            assert!(
                timeout(Duration::from_millis(30), receive_load(&mut receiver))
                    .await
                    .is_err()
            );
            // The backend may replace its account while activation is in
            // flight. The delayed request must retain the admitted token so
            // its caller can reject it, rather than relabelling it as 42.
            account_generation.store(42, Ordering::Release);
            release.send(()).unwrap();
            if lose_auth {
                timeout(Duration::from_secs(2), async {
                    loop {
                        match receiver.recv().await.unwrap() {
                            Update::Load { .. } => {
                                panic!("authentication loss must cancel the load")
                            }
                            Update::Ready(false) => break,
                            _ => {}
                        }
                    }
                })
                .await
                .unwrap();
                assert!(
                    timeout(Duration::from_millis(50), receive_load(&mut receiver))
                        .await
                        .is_err()
                );
                assert!(handle.command(PlayerCommand::Next).is_err());
            } else {
                assert_eq!(
                    timeout(Duration::from_secs(2), receive_load(&mut receiver))
                        .await
                        .unwrap(),
                    (spec, 41)
                );
            }
            drop(handle);
            server.abort();
        }
    }
}
