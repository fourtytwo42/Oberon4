#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayFrame {
    RequestOpen { request_id: String },
    RequestChunk { request_id: String, chunk: String },
    RequestEnd { request_id: String },
    ResponseChunk { request_id: String, chunk: String },
    ResponseEnd { request_id: String },
    Error { request_id: String, code: String },
    Ping,
    Pong,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayErrorKind {
    Disconnected,
    Timeout,
    Protocol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackoffState {
    pub attempts: u32,
}

impl BackoffState {
    pub fn next_delay_ms(&self) -> u64 {
        let pow = 2_u64.saturating_pow(self.attempts.min(10));
        100_u64.saturating_mul(pow)
    }

    pub fn increment(self) -> Self {
        Self {
            attempts: self.attempts.saturating_add(1),
        }
    }

    pub fn reset() -> Self {
        Self { attempts: 0 }
    }
}

pub fn assemble_chunks(frames: &[RelayFrame], request_id: &str) -> Result<String, RelayErrorKind> {
    let mut out = String::new();
    let mut ended = false;

    for frame in frames {
        match frame {
            RelayFrame::ResponseChunk {
                request_id: rid,
                chunk,
            } if rid == request_id => {
                out.push_str(chunk);
            }
            RelayFrame::ResponseEnd { request_id: rid } if rid == request_id => {
                ended = true;
                break;
            }
            RelayFrame::Error {
                request_id: rid, ..
            } if rid == request_id => return Err(RelayErrorKind::Protocol),
            _ => {}
        }
    }

    if ended {
        Ok(out)
    } else {
        Err(RelayErrorKind::Timeout)
    }
}
