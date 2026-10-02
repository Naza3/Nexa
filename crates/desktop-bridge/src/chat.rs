use crate::{
    BridgeError, ChatBatch, ChatEvent, ChatStartRequest, DesktopBridge, RequestHandle, Result,
    Stopping, sse::Decoder,
};
use http_body_util::BodyExt;
use hyper::{HeaderMap, Method};
use runtime_cli::client::RequestBody;
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::Duration,
};
use tokio::{sync::Notify, time::Instant};
use uuid::Uuid;
const QUEUE_BYTES: usize = 64 * 1024;
const BATCH_BYTES: usize = 16 * 1024;
const REPLY_BYTES: usize = 256 * 1024;
const HISTORY_BYTES: usize = 512 * 1024;
const STALL: Duration = Duration::from_secs(10);
#[derive(Default)]
pub(crate) struct ChatSlot {
    current: Option<Arc<Session>>,
    previous: Option<(Uuid, ChatEvent)>,
}
struct Queue {
    events: VecDeque<ChatEvent>,
    bytes: usize,
    terminal: Option<ChatEvent>,
    last_progress: Instant,
}
struct Session {
    id: Uuid,
    queue: Mutex<Queue>,
    changed: Notify,
    space: Notify,
    cancel: Notify,
    cancelled: AtomicBool,
    consuming: AtomicBool,
    phase: AtomicU8,
}
impl Session {
    fn new(id: Uuid) -> Self {
        Self {
            id,
            queue: Mutex::new(Queue {
                events: VecDeque::new(),
                bytes: 0,
                terminal: None,
                last_progress: Instant::now(),
            }),
            changed: Notify::new(),
            space: Notify::new(),
            cancel: Notify::new(),
            cancelled: AtomicBool::new(false),
            consuming: AtomicBool::new(false),
            phase: AtomicU8::new(0),
        }
    }
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.cancel.notify_waiters();
    }
    async fn cancelled(&self) {
        loop {
            let notified = self.cancel.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.cancelled.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
    fn finish(&self, event: ChatEvent) {
        let mut q = self.queue.lock().unwrap();
        if q.terminal.is_some() {
            return;
        }
        q.terminal = Some(event);
        self.phase.store(3, Ordering::Release);
        drop(q);
        self.changed.notify_waiters();
        self.space.notify_waiters();
    }
    async fn enqueue(&self, event: ChatEvent) -> Result<()> {
        let size = match &event {
            ChatEvent::Delta { text } => text.len(),
            _ => 0,
        };
        if size > QUEUE_BYTES {
            return Err(BridgeError::new("stream_invalid"));
        }
        loop {
            let notified = self.space.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let deadline = {
                let mut q = self.queue.lock().unwrap();
                if q.terminal.is_some() {
                    return Err(BridgeError::new("stream_invalid"));
                }
                if q.bytes + size <= QUEUE_BYTES && q.events.len() < 32 {
                    if q.events.is_empty() {
                        q.last_progress = Instant::now();
                    }
                    q.bytes += size;
                    match (q.events.back_mut(), &event) {
                        (Some(ChatEvent::Delta { text: old }), ChatEvent::Delta { text }) => {
                            old.push_str(text)
                        }
                        _ => q.events.push_back(event),
                    }
                    drop(q);
                    self.changed.notify_waiters();
                    return Ok(());
                }
                q.last_progress + STALL
            };
            tokio::select! { biased; _=self.cancelled()=>return Err(BridgeError::new("request_cancelled")),_=tokio::time::sleep_until(deadline)=>return Err(BridgeError::new("slow_consumer")),_=notified=>() }
        }
    }
    fn stall_deadline(&self) -> Instant {
        let q = self.queue.lock().unwrap();
        if q.events.is_empty() {
            Instant::now() + STALL
        } else {
            q.last_progress + STALL
        }
    }
    fn stalled(&self) -> bool {
        let q = self.queue.lock().unwrap();
        !q.events.is_empty() && q.last_progress.elapsed() >= STALL
    }
    fn batch(&self) -> ChatBatch {
        let mut q = self.queue.lock().unwrap();
        let mut events = Vec::new();
        let mut bytes = 0;
        while let Some(event) = q.events.pop_front() {
            match event {
                ChatEvent::Delta { text } => {
                    let available = BATCH_BYTES - bytes;
                    let mut cut = text.len().min(available);
                    while !text.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    if cut == 0 {
                        q.events.push_front(ChatEvent::Delta { text });
                        break;
                    }
                    events.push(ChatEvent::Delta {
                        text: text[..cut].into(),
                    });
                    bytes += cut;
                    q.bytes -= cut;
                    if cut < text.len() {
                        q.events.push_front(ChatEvent::Delta {
                            text: text[cut..].into(),
                        });
                        break;
                    }
                }
                other => events.push(other),
            }
            if bytes >= BATCH_BYTES || events.len() >= 32 {
                break;
            }
        }
        if !events.is_empty() {
            q.last_progress = Instant::now();
            self.space.notify_waiters();
        }
        let terminal = q.events.is_empty() && q.terminal.is_some();
        if terminal {
            events.push(q.terminal.clone().unwrap());
        }
        ChatBatch {
            request_id: self.id,
            events,
            terminal,
        }
    }
}
struct Consumer<'a>(&'a AtomicBool);
impl Drop for Consumer<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl DesktopBridge {
    /// Registers the ID and claims this window's slot before spawning any I/O.
    pub fn chat_start(self: &Arc<Self>, request: ChatStartRequest) -> Result<RequestHandle> {
        self.open()?;
        runtime_types::ModelId::new(&request.model_id)
            .map_err(|_| BridgeError::new("invalid_request"))?;
        runtime_types::validate_messages(&request.messages)
            .map_err(|_| BridgeError::new("invalid_request"))?;
        if !(1..=runtime_types::MAX_OUTPUT_TOKENS).contains(&request.max_output_tokens) {
            return Err(BridgeError::new("invalid_request"));
        }
        let history: usize = request.messages.iter().map(|m| m.content.len()).sum();
        if history >= HISTORY_BYTES
            || request.messages.len() >= 128
            || serde_json::to_vec(&request)
                .map_err(|_| BridgeError::new("invalid_request"))?
                .len()
                > HISTORY_BYTES
        {
            return Err(BridgeError::new("history_limit"));
        }
        let body=serde_json::to_vec(&json!({"model":request.model_id,"messages":request.messages,"stream":true,"stream_options":{"include_usage":true},"max_tokens":request.max_output_tokens})).map_err(|_|BridgeError::new("invalid_request"))?;
        if body.len() > HISTORY_BYTES {
            return Err(BridgeError::new("history_limit"));
        }
        let mut slot = self.chat.lock().unwrap();
        self.open()?;
        if let Some(current) = slot.current.as_ref() {
            let q = current.queue.lock().unwrap();
            if q.terminal.is_none()
                || !q.events.is_empty()
                || current.consuming.load(Ordering::Acquire)
            {
                return Err(BridgeError::new("desktop_busy"));
            }
            let previous = (current.id, q.terminal.clone().unwrap());
            drop(q);
            slot.previous = Some(previous);
        }
        let session = Arc::new(Session::new(Uuid::new_v4()));
        let id = session.id;
        slot.current = Some(session.clone());
        drop(slot);
        let bridge = self.clone();
        tokio::spawn(async move {
            bridge
                .run_chat(session, request.model_id, body, HISTORY_BYTES - history)
                .await;
        });
        Ok(RequestHandle { request_id: id })
    }
    pub async fn chat_next(&self, request_id: Uuid) -> Result<ChatBatch> {
        let session = {
            let slot = self.chat.lock().unwrap();
            if let Some(s) = slot.current.as_ref().filter(|s| s.id == request_id) {
                s.clone()
            } else if let Some((id, event)) =
                slot.previous.as_ref().filter(|(id, _)| *id == request_id)
            {
                return Ok(ChatBatch {
                    request_id: *id,
                    events: vec![event.clone()],
                    terminal: true,
                });
            } else {
                return Err(BridgeError::new("request_not_owned"));
            }
        };
        if session
            .consuming
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(BridgeError::new("consumer_busy"));
        }
        let _consumer = Consumer(&session.consuming);
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let notified = session.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let batch = session.batch();
            if !batch.events.is_empty() || batch.terminal {
                return Ok(batch);
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return Ok(session.batch());
            }
        }
    }
    pub async fn chat_cancel(&self, request_id: Uuid) -> Result<Stopping> {
        let slot = self.chat.lock().unwrap();
        if let Some(s) = slot.current.as_ref().filter(|s| s.id == request_id) {
            s.cancel();
        } else if !slot
            .previous
            .as_ref()
            .is_some_and(|(id, _)| *id == request_id)
        {
            return Err(BridgeError::new("request_not_owned"));
        }
        Ok(Stopping {
            request_id,
            status: "stopping",
        })
    }
    async fn send_cancel(&self, id: Uuid) {
        // A 404/202 is never promoted to a completed generation. Closing the
        // owned streaming socket is the independent disconnect-cancel path.
        let _ = tokio::time::timeout(Duration::from_secs(5), async {
            let mut connection = self.connect().await?;
            connection
                .json(
                    Method::POST,
                    &format!("/runtime/requests/{id}/cancel"),
                    Some(&json!({})),
                )
                .await
                .map_err(BridgeError::from)
        })
        .await;
    }
    async fn run_chat(
        &self,
        session: Arc<Session>,
        model: String,
        body: Vec<u8>,
        remaining: usize,
    ) {
        let result = self.stream_chat(&session, model, body, remaining).await;
        if session.phase.load(Ordering::Acquire) == 1 {
            // A cold external-model chat may still own preflight work before
            // response headers. Closing this UI must observe that lease drain.
            self.load_disconnected.store(true, Ordering::Release);
        }
        // stream_chat owns and drops the verified connection before this point.
        // Cleanup stays out of the UI work lock and cannot lose an early cancel.
        let event = match result {
            Ok(event) if !session.cancelled.load(Ordering::Acquire) => event,
            Ok(_) | Err(_) if session.cancelled.load(Ordering::Acquire) => {
                if session.phase.load(Ordering::Acquire) > 0 {
                    self.send_cancel(session.id).await;
                }
                ChatEvent::Cancelled
            }
            Err(error) => {
                if session.phase.load(Ordering::Acquire) > 0 {
                    self.send_cancel(session.id).await;
                }
                ChatEvent::Failed {
                    code: error.code,
                    message: error.message,
                }
            }
            Ok(event) => event,
        };
        session.finish(event);
    }
    async fn stream_chat(
        &self,
        session: &Session,
        model: String,
        body: Vec<u8>,
        remaining: usize,
    ) -> Result<ChatEvent> {
        let mut connection = tokio::select! {biased;_=session.cancelled()=>return Ok(ChatEvent::Cancelled),result=self.connect()=>result?};
        if session.cancelled.load(Ordering::Acquire) {
            return Ok(ChatEvent::Cancelled);
        }
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-request-id",
            session
                .id
                .to_string()
                .parse()
                .map_err(|_| BridgeError::new("invalid_request"))?,
        );
        let response = tokio::select! {biased;
            _=session.cancelled()=>return Ok(ChatEvent::Cancelled),
            result=async {
                if session.cancelled.load(Ordering::Acquire){return Err(BridgeError::new("request_cancelled"));}
                // No await between this send boundary and polling the fixed
                // request. An earlier cancel never dispatches the generation.
                session.phase.store(1,Ordering::Release);
                connection.request(Method::POST,"/v1/chat/completions",RequestBody::fixed(body),headers).await.map_err(BridgeError::from)
            }=>result?
        };
        if response.status() != 200 {
            let bytes = tokio::select! {biased;_=session.cancelled()=>return Ok(ChatEvent::Cancelled),result=runtime_cli::client::collect_bounded(response.into_body(),32*1024,Duration::from_secs(5))=>result?};
            let value = runtime_api::dto::parse_json(&bytes).ok();
            return Err(BridgeError::api(
                value
                    .as_ref()
                    .and_then(|v| v.get("error"))
                    .and_then(|e| e.get("code"))
                    .and_then(|v| v.as_str()),
            ));
        }
        if response
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .is_none_or(|s| s.split(';').next() != Some("text/event-stream"))
        {
            return Err(BridgeError::new("stream_invalid"));
        }
        session.phase.store(2, Ordering::Release);
        let mut body = response.into_body();
        let mut decoder = Decoder::new(session.id, model);
        let mut total = 0;
        let deadline = Instant::now() + Duration::from_secs(750);
        loop {
            if let Some(terminal) =
                drain_events(session, &mut decoder, &mut total, remaining).await?
            {
                return Ok(terminal);
            }
            let stall = session.stall_deadline();
            tokio::select! {biased;
                _=session.cancelled()=>return Ok(ChatEvent::Cancelled),
                _=tokio::time::sleep_until(deadline)=>return Err(BridgeError::new("execution_timeout")),
                _=tokio::time::sleep_until(stall)=>{if session.stalled(){return Err(BridgeError::new("slow_consumer"));}},
                frame=body.frame()=>{
                    let frame=frame.ok_or_else(||BridgeError::new("stream_invalid"))?.map_err(|_|BridgeError::new("stream_invalid"))?;
                    if let Ok(data)=frame.into_data(){
                        // Hyper frames are transport units, never application
                        // event limits. Parse between bounded slices even if
                        // one frame contains many valid SSE events.
                        for part in data.chunks(16*1024) {
                            decoder.push(part)?;
                            if let Some(terminal)=drain_events(session,&mut decoder,&mut total,remaining).await? { return Ok(terminal); }
                        }
                    }
                }
            }
        }
    }
    pub(crate) async fn close_chat(&self) -> Result<()> {
        let session = self.chat.lock().unwrap().current.clone();
        if let Some(session) = session {
            session.cancel();
            tokio::time::timeout(Duration::from_secs(7), async {
                loop {
                    let changed = session.changed.notified();
                    tokio::pin!(changed);
                    changed.as_mut().enable();
                    if session.queue.lock().unwrap().terminal.is_some() {
                        break;
                    }
                    changed.await;
                }
            })
            .await
            .map_err(|_| BridgeError::new("request_cleanup_unconfirmed"))?;
        }
        Ok(())
    }
}

async fn drain_events(
    session: &Session,
    decoder: &mut Decoder,
    total: &mut usize,
    remaining: usize,
) -> Result<Option<ChatEvent>> {
    while let Some(event) = decoder.next()? {
        if event.is_terminal() {
            return Ok(Some(event));
        }
        if let ChatEvent::Delta { text } = &event {
            *total += text.len();
            if *total > REPLY_BYTES {
                return Err(BridgeError::new("response_limit"));
            }
            if *total > remaining {
                return Err(BridgeError::new("history_limit"));
            }
        }
        session.enqueue(event).await?;
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn bounded_utf8_batches_terminal_once_and_repeat_summary() {
        let s = Session::new(Uuid::new_v4());
        s.enqueue(ChatEvent::Started).await.unwrap();
        let text = "你好😀".repeat(3000);
        s.enqueue(ChatEvent::Delta { text: text.clone() })
            .await
            .unwrap();
        s.finish(ChatEvent::Cancelled);
        s.finish(ChatEvent::Failed {
            code: "bad".into(),
            message: "bad".into(),
        });
        let mut all = String::new();
        loop {
            let b = s.batch();
            let bytes: usize = b
                .events
                .iter()
                .map(|e| {
                    if let ChatEvent::Delta { text } = e {
                        all.push_str(text);
                        text.len()
                    } else {
                        0
                    }
                })
                .sum();
            assert!(bytes <= BATCH_BYTES);
            if b.terminal {
                assert_eq!(b.events.last(), Some(&ChatEvent::Cancelled));
                break;
            }
        }
        assert_eq!(all, text);
        assert_eq!(s.batch().events, vec![ChatEvent::Cancelled]);
    }
    #[tokio::test]
    async fn queue_backpressure_can_be_cancelled() {
        let s = Arc::new(Session::new(Uuid::new_v4()));
        s.enqueue(ChatEvent::Delta {
            text: "a".repeat(QUEUE_BYTES),
        })
        .await
        .unwrap();
        let cloned = s.clone();
        let task =
            tokio::spawn(
                async move { cloned.enqueue(ChatEvent::Delta { text: "x".into() }).await },
            );
        tokio::task::yield_now().await;
        assert!(!task.is_finished());
        s.cancel();
        assert!(task.await.unwrap().is_err());
    }
    #[tokio::test]
    async fn stale_consumer_fails_without_growing_queue() {
        let s = Session::new(Uuid::new_v4());
        s.enqueue(ChatEvent::Delta {
            text: "a".repeat(QUEUE_BYTES),
        })
        .await
        .unwrap();
        s.queue.lock().unwrap().last_progress = Instant::now() - STALL;
        assert_eq!(
            s.enqueue(ChatEvent::Delta { text: "x".into() })
                .await
                .unwrap_err()
                .code,
            "slow_consumer"
        );
        assert_eq!(s.queue.lock().unwrap().bytes, QUEUE_BYTES);
    }
}
