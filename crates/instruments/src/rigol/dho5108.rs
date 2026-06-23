use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

enum DhoTransport {
    Tcp(BufReader<TcpStream>),
    #[cfg(target_os = "windows")]
    Visa(gpib_rs::Instrument),
}

pub struct DHO5108 {
    transport: DhoTransport,
}

#[derive(Debug, Clone)]
pub struct DhoWaveformPreamble {
    pub raw: String,
    pub x_increment: f64,
    pub x_origin: f64,
    pub x_reference: f64,
    pub y_increment: f64,
    pub y_origin: f64,
    pub y_reference: f64,
    pub vertical_offset: f64,
    pub vertical_scale: f64,
}

#[derive(Debug, Clone)]
pub struct DhoRawWaveform {
    pub preamble: DhoWaveformPreamble,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct DhoRawWaveformWritten {
    pub preamble: DhoWaveformPreamble,
    pub byte_count: usize,
}

#[allow(dead_code)]
impl DHO5108 {
    pub fn open(ip: &str, port: u16, timeout: Option<Duration>) -> io::Result<Self> {
        let addr = format!("{}:{}", ip, port);
        let stream = TcpStream::connect(addr)?;
        stream.set_read_timeout(timeout)?;
        stream.set_write_timeout(timeout)?;
        stream.set_nodelay(true)?;
        let reader = BufReader::new(stream);
        Ok(DHO5108 {
            transport: DhoTransport::Tcp(reader),
        })
    }

    pub fn open_usbtmc(resource: &str, timeout: Option<Duration>) -> io::Result<Self> {
        #[cfg(target_os = "windows")]
        {
            let instrument = gpib_rs::Instrument::open_resource(resource, timeout)
                .map_err(|error| io::Error::other(error.to_string()))?;
            Ok(Self {
                transport: DhoTransport::Visa(instrument),
            })
        }

        #[cfg(not(target_os = "windows"))]
        {
            let _ = (resource, timeout);
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "USB-TMC currently requires NI-VISA on Windows",
            ))
        }
    }

    fn close(self) {}

    fn write_raw(&mut self, data: &[u8]) -> io::Result<()> {
        match &mut self.transport {
            DhoTransport::Tcp(reader) => {
                reader.get_mut().write_all(data)?;
                reader.get_mut().flush()
            }
            #[cfg(target_os = "windows")]
            DhoTransport::Visa(instrument) => instrument
                .write_raw(data)
                .map_err(|error| io::Error::other(error.to_string())),
        }
    }

    fn tcp_reader_mut(&mut self) -> &mut BufReader<TcpStream> {
        #[cfg(target_os = "windows")]
        match &mut self.transport {
            DhoTransport::Tcp(reader) => reader,
            DhoTransport::Visa(_) => unreachable!("VISA transport is handled separately"),
        }

        #[cfg(not(target_os = "windows"))]
        {
            let DhoTransport::Tcp(reader) = &mut self.transport;
            reader
        }
    }

    pub fn write_line(&mut self, cmd: &str) -> io::Result<()> {
        let s = format!("{cmd}\n");
        self.write_raw(s.as_bytes())
    }

    fn write_lines(&mut self, commands: &[String]) -> io::Result<()> {
        let capacity = commands.iter().map(|command| command.len() + 1).sum();
        let mut request = String::with_capacity(capacity);
        for command in commands {
            request.push_str(command);
            request.push('\n');
        }
        self.write_raw(request.as_bytes())
    }

    pub fn read_line(&mut self) -> io::Result<String> {
        match &mut self.transport {
            DhoTransport::Tcp(reader) => {
                let mut s = String::new();
                for _ in 0..4 {
                    s.clear();
                    let read = reader.read_line(&mut s)?;
                    let trimmed = s.trim();
                    if read == 0 || !trimmed.is_empty() {
                        return Ok(trimmed.to_string());
                    }
                }
                Ok(String::new())
            }
            #[cfg(target_os = "windows")]
            DhoTransport::Visa(instrument) => instrument
                .read_string()
                .map_err(|error| io::Error::other(error.to_string())),
        }
    }

    pub fn query(&mut self, cmd: &str) -> io::Result<String> {
        self.write_line(cmd)?;
        self.read_line()
    }

    pub fn query_binary(&mut self, cmd: &str) -> io::Result<Vec<u8>> {
        #[cfg(target_os = "windows")]
        if let DhoTransport::Visa(instrument) = &mut self.transport {
            let mut data = Vec::new();
            instrument
                .query_ieee_block(cmd, &mut data)
                .map_err(|error| io::Error::other(error.to_string()))?;
            return Ok(data);
        }

        self.write_line(cmd)?;

        let reader = self.tcp_reader_mut();
        let length = read_binary_block_length(reader)?;

        let mut data = vec![0u8; length];
        reader.read_exact(&mut data)?;
        consume_buffered_terminator(reader);

        Ok(data)
    }

    pub fn query_binary_into<W: Write>(&mut self, cmd: &str, writer: &mut W) -> io::Result<usize> {
        #[cfg(target_os = "windows")]
        if let DhoTransport::Visa(instrument) = &mut self.transport {
            let mut data = Vec::new();
            instrument
                .query_ieee_block(cmd, &mut data)
                .map_err(|error| io::Error::other(error.to_string()))?;
            writer.write_all(&data)?;
            return Ok(data.len());
        }

        self.write_line(cmd)?;

        let reader = self.tcp_reader_mut();
        let length = read_binary_block_length(reader)?;
        let copied = {
            let mut limited = reader.by_ref().take(length as u64);
            io::copy(&mut limited, writer)?
        };
        if copied != length as u64 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("binary block ended after {copied} bytes, expected {length}"),
            ));
        }
        consume_buffered_terminator(reader);
        Ok(length)
    }

    pub fn identify(&mut self) -> io::Result<String> {
        self.query("*IDN?")
    }

    fn setup_raw_word_fetch(&mut self, ch: u8, memory_depth: usize) -> io::Result<()> {
        // Send sequential setup commands in one write and synchronize once at the end.
        self.write_lines(&[
            format!("WAV:SOUR CHAN{ch}"),
            "WAV:MODE RAW".to_string(),
            "WAV:FORM WORD".to_string(),
            "WAV:STAR 1".to_string(),
            format!("WAV:POIN {memory_depth}"),
            format!("WAV:STOP {memory_depth}"),
            "*OPC?".to_string(),
        ])?;
        let _ = self.read_line()?; // "1"
        Ok(())
    }

    fn query_waveform_preamble(&mut self, ch: u8) -> io::Result<DhoWaveformPreamble> {
        // PREamble preserves the full instrument context in metadata, but Rigol
        // rounds some scaling fields there. Query voltage scaling separately so
        // CSV/raw replay matches the older high-precision conversion path.
        let preamble = self.query("WAV:PRE?")?;
        let fields: Vec<&str> = preamble.split(',').collect();
        if fields.len() != 10 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("expected 10 waveform preamble fields, got {}", fields.len()),
            ));
        }
        let parse_field = |index: usize, name: &str| -> io::Result<f64> {
            fields[index].parse().map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid waveform preamble {name}: {error}"),
                )
            })
        };
        let x_increment = parse_field(4, "xincrement")?;
        let x_origin = parse_field(5, "xorigin")?;
        let x_reference = parse_field(6, "xreference")?;
        let y_increment = self.query_f64("WAV:YINC?", "yincrement")?;
        let y_origin = self.query_f64("WAV:YOR?", "yorigin")?;
        let y_reference = self.query_f64("WAV:YREF?", "yreference")?;
        let vertical_offset =
            self.query_f64(&format!(":CHANnel{ch}:OFFSet?"), "channel vertical offset")?;
        let vertical_scale =
            self.query_f64(&format!(":CHANnel{ch}:SCALe?"), "channel vertical scale")?;

        Ok(DhoWaveformPreamble {
            raw: preamble,
            x_increment,
            x_origin,
            x_reference,
            y_increment,
            y_origin,
            y_reference,
            vertical_offset,
            vertical_scale,
        })
    }

    fn query_f64(&mut self, cmd: &str, name: &str) -> io::Result<f64> {
        self.query(cmd)?.parse().map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid waveform {name}: {error}"),
            )
        })
    }

    pub fn fetch_raw_word(&mut self, ch: u8, memory_depth: usize) -> io::Result<DhoRawWaveform> {
        self.setup_raw_word_fetch(ch, memory_depth)?;
        let preamble = self.query_waveform_preamble(ch)?;

        let data = self.query_binary("WAV:DATA?")?;

        if data.len() % 2 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "odd length binary",
            ));
        }

        Ok(DhoRawWaveform { preamble, data })
    }

    pub fn fetch_raw_word_into<W: Write>(
        &mut self,
        ch: u8,
        memory_depth: usize,
        writer: &mut W,
    ) -> io::Result<DhoRawWaveformWritten> {
        self.setup_raw_word_fetch(ch, memory_depth)?;
        let preamble = self.query_waveform_preamble(ch)?;

        let byte_count = self.query_binary_into("WAV:DATA?", writer)?;

        if byte_count % 2 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "odd length binary",
            ));
        }

        Ok(DhoRawWaveformWritten {
            preamble,
            byte_count,
        })
    }

    pub fn fetch(&mut self, ch: u8, memory_depth: usize) -> io::Result<Vec<f64>> {
        let raw = self.fetch_raw_word(ch, memory_depth)?;
        let y_inc = raw.preamble.y_increment;
        let y_ori = raw.preamble.y_origin;
        let y_ref = raw.preamble.y_reference;

        let mut result = Vec::with_capacity(raw.data.len() / 2);
        for chunk in raw.data.chunks_exact(2) {
            let v = u16::from_le_bytes([chunk[0], chunk[1]]) as f64;
            let y = (v - y_ori - y_ref) * y_inc;
            result.push(y);
        }

        Ok(result)
    }

    pub fn set_single(&mut self) -> io::Result<()> {
        self.write_line("TRIG:SWE SING")?;
        Ok(())
    }
}

fn read_binary_block_length<R: BufRead>(reader: &mut R) -> io::Result<usize> {
    // SCPI binary block structure:
    // 1  1         n             length          1
    // # <n> <length_header> <binary_data> [<terminator>]
    let start = read_next_non_terminator_byte(reader)?;
    if start != b'#' {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Expected '#' at the start of binary block",
        ));
    }

    let mut one = [0u8; 1];
    reader.read_exact(&mut one)?;
    if !one[0].is_ascii_digit() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "binary block length digit is not ASCII",
        ));
    }
    let n = (one[0] - b'0') as usize;

    let mut len_buf = vec![0u8; n];
    reader.read_exact(&mut len_buf)?;
    let len_str =
        String::from_utf8(len_buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    len_str
        .parse()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn read_next_non_terminator_byte<R: BufRead>(reader: &mut R) -> io::Result<u8> {
    let mut one = [0u8; 1];
    loop {
        reader.read_exact(&mut one)?;
        if !matches!(one[0], b'\r' | b'\n') {
            return Ok(one[0]);
        }
    }
}

fn consume_buffered_terminator(reader: &mut BufReader<TcpStream>) -> bool {
    let buffered = reader.buffer();
    let consume = match buffered.first().copied() {
        Some(b'\n') => 1,
        Some(b'\r') if buffered.get(1) == Some(&b'\n') => 2,
        Some(b'\r') => 1,
        _ => 0,
    };

    if consume > 0 {
        reader.consume(consume);
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor, Read};

    #[test]
    fn binary_block_length_skips_leftover_line_terminators() {
        let cursor = Cursor::new(b"\r\n#14abcd".to_vec());
        let mut reader = BufReader::new(cursor);

        let length = read_binary_block_length(&mut reader).unwrap();
        let mut payload = vec![0; length];
        reader.read_exact(&mut payload).unwrap();

        assert_eq!(length, 4);
        assert_eq!(payload, b"abcd");
    }

    #[test]
    fn binary_block_length_rejects_non_block_data() {
        let cursor = Cursor::new(b"not a block".to_vec());
        let mut reader = BufReader::new(cursor);

        let error = read_binary_block_length(&mut reader).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
