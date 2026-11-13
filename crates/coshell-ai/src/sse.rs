//! Minimal Server-Sent Events parser (byte-oriented, so UTF-8 sequences split
//! across network chunks are handled).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
}

impl SseParser {
    /// Feed bytes; returns every event completed by them.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(nl) = self.buf.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=nl).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line).into_owned();
            if line.is_empty() {
                if !self.data.is_empty() || self.event.is_some() {
                    out.push(SseEvent { event: self.event.take(), data: std::mem::take(&mut self.data).join("\n") });
                }
                continue;
            }
            if line.starts_with(':') {
                continue; // comment / keep-alive
            }
            let (field, value) = match line.split_once(':') {
                Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
                None => (line.as_str(), ""),
            };
            match field {
                "event" => self.event = Some(value.to_string()),
                "data" => self.data.push(value.to_string()),
                _ => {}
            }
        }
        out
    }

    /// Flush a trailing event not terminated by a blank line.
    pub fn finish(&mut self) -> Option<SseEvent> {
        let tail = std::mem::take(&mut self.buf);
        if !tail.is_empty() {
            let mut evs = self.push(&[tail.as_slice(), b"\n"].concat());
            if let Some(e) = evs.pop() {
                return Some(e);
            }
        }
        (!self.data.is_empty()).then(|| SseEvent { event: self.event.take(), data: std::mem::take(&mut self.data).join("\n") })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_across_chunks_and_split_utf8() {
        let raw = "event: a\ndata: 안녕\n\n: ping\n\ndata: x\ndata: y\r\n\r\n".as_bytes();
        let mut p = SseParser::default();
        let mut got = Vec::new();
        for chunk in raw.chunks(3) {
            got.extend(p.push(chunk));
        }
        assert_eq!(
            got,
            [
                SseEvent { event: Some("a".into()), data: "안녕".into() },
                SseEvent { event: None, data: "x\ny".into() }
            ]
        );
    }

    #[test]
    fn finish_flushes_unterminated_event() {
        let mut p = SseParser::default();
        assert!(p.push(b"data: [DONE]").is_empty());
        assert_eq!(p.finish(), Some(SseEvent { event: None, data: "[DONE]".into() }));
    }
}
