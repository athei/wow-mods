//! Pending-request / completed-result queues for asynchronous translation.
//!
//! Translation flow:
//! 1. Hook entry pushes a [`Request`] via [`enqueue`], which is non-blocking
//!    and bounded — the Lua thread never waits on the worker, and a worker that
//!    has stopped draining sheds requests instead of accumulating them. The
//!    first push lazy-spawns a worker thread (no thread spawn from `DllMain`,
//!    per the project's `DllMain` rules).
//! 2. The worker blocks on `mpsc::Receiver::recv`, then drains everything else
//!    already queued into one batch, groups it by language pair, and issues one
//!    `Translate` thunk per group (Apple's session API is per-pair). Each
//!    item's UTF-8 result is parked into the completed channel.
//! 3. Hook `poll` calls [`drain_completed`] which pulls every ready result so a
//!    burst surfaces in a single Lua tick.
//!
//! No drop path: at process exit the receiver-held thread is reaped by the
//! OS. The `LazyLock` holding the `SyncSender` never drops, so the channel stays
//! open. Matches `[[feedback_no_terminateprocess_in_release]]`.

use std::sync::{LazyLock, Mutex, mpsc};

use wow_shared::{TranslateParams, TranslateStatus};

use crate::{LOG_TARGET, unix_call};

/// 16 KiB per output item.
///
/// Apple Translation refuses inputs above 1000 codepoints anyway; chat messages
/// are well under that.
const OUT_SLOT_BYTES: usize = 16 * 1024;

/// Cap on how many queued requests one batch coalesces.
///
/// Bounds the transient output allocation (`MAX_BATCH * OUT_SLOT_BYTES`) and the
/// Swift-side batch.
const MAX_BATCH: usize = 32;

/// Depth of the pending-request queue: four coalesced batches.
///
/// Deep enough that no realistic chat burst reaches it, shallow enough that a
/// wedged unix side sheds load. Without a bound, a worker stuck in one `Translate`
/// thunk lets the addon's 25 s request timeout free its in-flight slots and
/// enqueue another round, forever, none of which anyone is waiting for any more.
const WORK_QUEUE_DEPTH: usize = 4 * MAX_BATCH;

pub struct Request {
    pub req_id: String,
    pub text: String,
    pub src_lang: String,
    pub tgt_lang: String,
}

pub struct Completed {
    pub req_id: String,
    pub translation: String,
    pub error: String,
}

/// Why [`enqueue`] turned a request away.
pub enum EnqueueError {
    /// The worker is a full [`WORK_QUEUE_DEPTH`] behind.
    ///
    /// The unix side is wedged or very slow. Transient: the addon shows the
    /// untranslated line and the next message tries again.
    Full,
    /// The worker thread is gone, so nothing will ever drain the queue.
    ///
    /// The addon latches `dllAvailable = false` on this one.
    Closed,
}

/// Both directions live behind one `LazyLock`.
///
/// The worker thread and the two channels are constructed atomically.
/// `mpsc::Receiver` is `!Sync`, hence the `Mutex` on `done_rx`; the hook only
/// polls from one thread anyway (Lua is single-threaded), so the lock is
/// effectively uncontended.
struct Channels {
    work_tx: mpsc::SyncSender<Request>,
    done_rx: Mutex<mpsc::Receiver<Completed>>,
}

static CHANNELS: LazyLock<Channels> = LazyLock::new(|| {
    let (work_tx, work_rx) = mpsc::sync_channel::<Request>(WORK_QUEUE_DEPTH);
    let (done_tx, done_rx) = mpsc::channel::<Completed>();
    std::thread::Builder::new()
        .name("wow_translate-worker".to_owned())
        .spawn(move || worker(&work_rx, &done_tx))
        .expect("spawn wow_translate worker");
    log::info!(target: LOG_TARGET, "spawned translator worker thread");
    Channels {
        work_tx,
        done_rx: Mutex::new(done_rx),
    }
});

/// Hand a request to the worker.
///
/// Never blocks: this runs on the game's Lua thread, so a saturated queue sheds
/// the request rather than stalling a frame.
pub fn enqueue(req: Request) -> Result<(), EnqueueError> {
    CHANNELS.work_tx.try_send(req).map_err(|e| match e {
        mpsc::TrySendError::Full(_) => EnqueueError::Full,
        mpsc::TrySendError::Disconnected(_) => EnqueueError::Closed,
    })
}

/// Drain up to `max` completed results in one lock acquisition.
///
/// The hook's `poll` can surface a whole burst in a single Lua tick.
pub fn drain_completed(max: usize) -> Vec<Completed> {
    let mut out = Vec::new();
    if let Ok(rx) = CHANNELS.done_rx.lock() {
        while out.len() < max {
            match rx.try_recv() {
                Ok(c) => out.push(c),
                Err(_) => break,
            }
        }
    }
    out
}

fn worker(work_rx: &mpsc::Receiver<Request>, done_tx: &mpsc::Sender<Completed>) {
    while let Ok(first) = work_rx.recv() {
        // Coalesce everything else already queued into one batch.
        let mut batch = vec![first];
        while batch.len() < MAX_BATCH {
            match work_rx.try_recv() {
                Ok(req) => batch.push(req),
                Err(_) => break,
            }
        }
        // One thunk call per language pair (Apple's session API is per-pair).
        for group in group_by_pair(batch) {
            for completed in translate_group(&group) {
                if done_tx.send(completed).is_err() {
                    return;
                }
            }
        }
    }
}

/// Partition a coalesced batch into runs that share a `(src_lang, tgt_lang)` pair.
///
/// Arrival order is preserved within each run. Chat is usually one pair, so this
/// is almost always a single group; a linear scan is cheap at batch size.
fn group_by_pair(batch: Vec<Request>) -> Vec<Vec<Request>> {
    let mut groups: Vec<Vec<Request>> = Vec::new();
    for req in batch {
        if let Some(g) = groups
            .iter_mut()
            .find(|g| g[0].src_lang == req.src_lang && g[0].tgt_lang == req.tgt_lang)
        {
            g.push(req);
        } else {
            groups.push(vec![req]);
        }
    }
    groups
}

/// Translate one same-pair group in a single batched thunk call.
///
/// Returns a [`Completed`] per request in input order.
fn translate_group(reqs: &[Request]) -> Vec<Completed> {
    let count = reqs.len();
    let src_lang = &reqs[0].src_lang;
    let tgt_lang = &reqs[0].tgt_lang;

    // Pack inputs: a per-item length array plus the concatenated UTF-8 bytes.
    let mut in_lens: Vec<u32> = Vec::with_capacity(count);
    let mut in_bytes: Vec<u8> = Vec::new();
    for r in reqs {
        in_lens.push(u32::try_from(r.text.len()).unwrap_or(u32::MAX));
        in_bytes.extend_from_slice(r.text.as_bytes());
    }

    // Outputs: one fixed `OUT_SLOT_BYTES` slot per item, plus per-item byte
    // counts and status. Default status `Internal` so a handler that returns
    // early surfaces as an error rather than a spurious `Ok`.
    let mut out_bytes = vec![0_u8; count * OUT_SLOT_BYTES];
    let mut out_lens = vec![0_u32; count];
    let mut out_status = vec![TranslateStatus::Internal as u32; count];

    let mut params = TranslateParams {
        src_lang_ptr: src_lang.as_ptr() as usize as u64,
        tgt_lang_ptr: tgt_lang.as_ptr() as usize as u64,
        in_lens_ptr: in_lens.as_ptr() as usize as u64,
        in_bytes_ptr: in_bytes.as_ptr() as usize as u64,
        out_bytes_ptr: out_bytes.as_mut_ptr() as usize as u64,
        out_lens_ptr: out_lens.as_mut_ptr() as usize as u64,
        out_status_ptr: out_status.as_mut_ptr() as usize as u64,
        count: u32::try_from(count).unwrap_or(u32::MAX),
        in_bytes_len: u32::try_from(in_bytes.len()).unwrap_or(u32::MAX),
        out_slot_bytes: u32::try_from(OUT_SLOT_BYTES).expect("16 KiB fits u32"),
        src_lang_len: u32::try_from(src_lang.len()).unwrap_or(u32::MAX),
        tgt_lang_len: u32::try_from(tgt_lang.len()).unwrap_or(u32::MAX),
        pad0: 0,
    };
    let _ = unix_call::call(&mut params);

    let mut out = Vec::with_capacity(count);
    for (i, r) in reqs.iter().enumerate() {
        let status = TranslateStatus::from_repr(out_status[i]).unwrap_or(TranslateStatus::Internal);
        let completed = if matches!(status, TranslateStatus::Ok) {
            let base = i * OUT_SLOT_BYTES;
            let n = (out_lens[i] as usize).min(OUT_SLOT_BYTES);
            core::str::from_utf8(&out_bytes[base..base + n]).map_or_else(
                |_| Completed {
                    req_id: r.req_id.clone(),
                    translation: String::new(),
                    error: "non-utf8 result".to_owned(),
                },
                |s| Completed {
                    req_id: r.req_id.clone(),
                    translation: s.to_owned(),
                    error: String::new(),
                },
            )
        } else {
            Completed {
                req_id: r.req_id.clone(),
                translation: String::new(),
                error: format!("{status:?}"),
            }
        };
        out.push(completed);
    }
    out
}
