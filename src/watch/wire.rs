use super::{Event, Result};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    time::{Duration, Instant},
};
pub const MAX: usize = 32 << 20;
pub const TTL: Duration = Duration::from_secs(30);
pub fn encode(event: &Event) -> Result<Vec<u8>> {
    let data = serde_json::to_vec(event).map_err(|e| e.to_string())?;
    if data.len() > MAX {
        return Err("watch publication exceeds 32 MiB".into());
    }
    Ok(data)
}
pub fn decode(data: &[u8]) -> Result<Event> {
    if data.len() > MAX {
        return Err("watch publication exceeds 32 MiB".into());
    }
    serde_json::from_slice(data).map_err(|e| format!("invalid watch publication: {e}"))
}
struct Parts {
    started: Instant,
    count: usize,
    size: usize,
    pieces: BTreeMap<usize, Vec<u8>>,
}
#[derive(Default)]
pub struct Receiver {
    partial: HashMap<uuid::Uuid, Parts>,
    recent: VecDeque<uuid::Uuid>,
    size: usize,
}
impl Receiver {
    pub fn deadline(&self) -> Option<Instant> {
        self.partial.values().map(|p| p.started + TTL).min()
    }
    pub fn accept(
        &mut self,
        id: uuid::Uuid,
        index: usize,
        count: usize,
        data: &[u8],
    ) -> Result<Option<Event>> {
        if count == 0 || count > MAX / 1024 + 1 || index >= count || data.is_empty() {
            return Err("invalid watch fragment".into());
        }
        if self.recent.contains(&id) {
            return Ok(None);
        }
        if self.partial.len() >= 4096 && !self.partial.contains_key(&id) {
            return Err("too many incomplete watch publications".into());
        }
        let p = self.partial.entry(id).or_insert_with(|| Parts {
            started: Instant::now(),
            count,
            size: 0,
            pieces: BTreeMap::new(),
        });
        if p.count != count {
            return Err("inconsistent watch fragment count".into());
        }
        if let Some(old) = p.pieces.get(&index) {
            return if old == data {
                Ok(None)
            } else {
                Err("inconsistent watch fragment data".into())
            };
        }
        if self.size + data.len() > MAX {
            return Err("watch assembly budget exceeded".into());
        }
        p.pieces.insert(index, data.to_vec());
        p.size += data.len();
        self.size += data.len();
        if p.pieces.len() != count {
            return Ok(None);
        }
        let done = self.partial.remove(&id).unwrap();
        self.size -= done.size;
        let mut data = Vec::with_capacity(done.size);
        for piece in done.pieces.into_values() {
            data.extend(piece)
        }
        let event = decode(&data)?;
        if self.recent.len() == 4096 {
            self.recent.pop_front();
        }
        self.recent.push_back(id);
        Ok(Some(event))
    }
}
pub async fn expiration(deadline: Option<Instant>) {
    if let Some(d) = deadline {
        tokio::time::sleep_until(d.into()).await
    } else {
        std::future::pending::<()>().await
    }
}
