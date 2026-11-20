//! Keep diagnostic formatting from issuing many tiny writes on the input loop.
use std::io::{self, Write};

/// Emit exactly the existing line protocol in one write when the sink accepts
/// the full buffer. `write_all` still handles interrupted/partial writes and
/// propagates failures; telemetry is never silently truncated or skipped.
pub fn write_sample(writer: &mut impl Write, sample: &serde_json::Value) -> io::Result<()> {
    let mut bytes = Vec::with_capacity(4096);
    bytes.extend_from_slice(b"BOT_SAMPLE ");
    serde_json::to_writer(&mut bytes, sample).map_err(io::Error::other)?;
    bytes.push(b'\n');
    writer.write_all(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sink {
        bytes: Vec<u8>,
        calls: usize,
        limit: usize,
        interrupt: bool,
    }

    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            if std::mem::take(&mut self.interrupt) {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let count = bytes.len().min(self.limit);
            self.bytes.extend_from_slice(&bytes[..count]);
            Ok(count)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn sink(limit: usize, interrupt: bool) -> Sink {
        Sink {
            bytes: Vec::new(),
            calls: 0,
            limit,
            interrupt,
        }
    }

    fn fixture() -> serde_json::Value {
        serde_json::json!({
            "nickname": "Игрок\n\"quoted\"\\路径", "phase": "running",
            "peers": (0..10).map(|id| serde_json::json!({"id":id,"position":[1.25,-0.5],
                "pending":null,"alive":true,"details":"x".repeat(1024)})).collect::<Vec<_>>()
        })
    }

    #[test]
    fn unchanged_large_record_uses_one_sink_write() {
        let sample = fixture();
        let mut previous = sink(usize::MAX, false);
        writeln!(&mut previous, "BOT_SAMPLE {sample}").unwrap();
        let mut current = sink(usize::MAX, false);
        write_sample(&mut current, &sample).unwrap();
        assert!(previous.bytes.len() > 4096);
        assert_eq!(current.bytes, previous.bytes);
        assert!(
            previous.calls > 20,
            "fixture must reproduce fragmented formatting"
        );
        assert_eq!(current.calls, 1);
        let line = std::str::from_utf8(&current.bytes).unwrap();
        assert_eq!(line.lines().count(), 1);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                line.trim_end().strip_prefix("BOT_SAMPLE ").unwrap()
            )
            .unwrap(),
            sample
        );
    }

    #[test]
    fn interrupted_and_short_writes_preserve_the_whole_record() {
        let sample = fixture();
        let expected = format!("BOT_SAMPLE {sample}\n");
        let mut writer = sink(7, true);
        write_sample(&mut writer, &sample).unwrap();
        assert_eq!(writer.bytes, expected.as_bytes());
        assert_eq!(writer.calls, expected.len().div_ceil(7) + 1);
    }

    #[test]
    fn zero_write_failure_is_not_reported_as_delivery() {
        let mut writer = sink(0, false);
        let error = write_sample(&mut writer, &fixture()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WriteZero);
        assert!(writer.bytes.is_empty());
    }
}
