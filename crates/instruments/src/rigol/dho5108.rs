use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub struct DHO5108 {
    reader: BufReader<TcpStream>,
}

#[allow(dead_code)]
impl DHO5108 {
    pub fn open(ip: &str, port: u16, timeout: Option<Duration>) -> io::Result<Self> {
        let addr = format!("{}:{}", ip, port);
        let stream = TcpStream::connect(addr)?;
        stream.set_read_timeout(timeout)?;
        stream.set_write_timeout(timeout)?;
        let reader = BufReader::new(stream);
        Ok(DHO5108 { reader })
    }

    fn inner_mut(&mut self) -> &mut TcpStream {
        self.reader.get_mut()
    }

    fn close(self) {}

    pub fn write_line(&mut self, cmd: &str) -> io::Result<()> {
        let s = format!("{cmd}\n");
        self.inner_mut().write_all(s.as_bytes())?;
        self.inner_mut().flush()?;
        Ok(())
    }

    pub fn read_line(&mut self) -> io::Result<String> {
        let mut s = String::new();
        self.reader.read_line(&mut s)?;
        Ok(s.trim().to_string())
    }

    pub fn query(&mut self, cmd: &str) -> io::Result<String> {
        self.write_line(cmd)?;
        self.read_line()
    }

    pub fn query_binary(&mut self, cmd: &str) -> io::Result<Vec<u8>> {
        self.write_line(cmd)?;

        // SCPI binary block structure:
        // 1  1         n             length          1
        // # <n> <length_header> <binary_data> [<terminator>]

        // SCPI binary block starts with '#'
        let mut one = [0u8; 1];
        self.reader.read_exact(&mut one)?;
        if one[0] != b'#' {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Expected '#' at the start of binary block",
            ));
        }

        self.reader.read_exact(&mut one)?;
        let n = (one[0] - b'0') as usize;

        let mut len_buf = vec![0u8; n];
        self.reader.read_exact(&mut len_buf)?;
        let len_str = String::from_utf8(len_buf)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let length: usize = len_str
            .parse()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let mut data = vec![0u8; length];
        self.reader.read_exact(&mut data)?;

        let _ = self.reader.read(&mut one);

        Ok(data)
    }

    pub fn identify(&mut self) -> io::Result<String> {
        self.query("*IDN?")
    }

    pub fn fetch(&mut self, ch: u8, memory_depth: usize) -> io::Result<Vec<f64>> {
        // Set data source to channel ch
        self.write_line(&format!("WAV:SOUR CHAN{ch}"))?;

        // Set Raw
        self.write_line("WAV:MODE RAW")?;
        let _ = self.write_line("*OPC?");
        let _ = self.read_line()?;

        // Set format to WORD, set start/stop points
        self.write_line("WAV:FORM WORD")?;
        self.write_line("WAV:STAR 1")?;
        self.write_line(&format!("WAV:POIN {memory_depth}"))?;
        self.write_line(&format!("WAV:STOP {memory_depth}"))?;
        self.write_line("*OPC?")?;
        let _ = self.read_line()?; // "1"

        // scale info
        let y_inc: f64 = self.query("WAV:YINC?")?.parse().unwrap();
        let y_ori: f64 = self.query("WAV:YOR?")?.parse().unwrap();
        let y_ref: f64 = self.query("WAV:YREF?")?.parse().unwrap();

        // read actual data
        let raw_bytes = self.query_binary("WAV:DATA?")?;

        if raw_bytes.len() % 2 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "odd length binary",
            ));
        }

        let mut result = Vec::with_capacity(raw_bytes.len() / 2);
        for chunk in raw_bytes.chunks_exact(2) {
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
