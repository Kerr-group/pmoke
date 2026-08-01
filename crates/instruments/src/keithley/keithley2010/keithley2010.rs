use crate::Result;
use crate::transport::BoxedScpiTransport;

pub struct Keithley2010 {
    transport: BoxedScpiTransport,
}

impl Keithley2010 {
    pub fn new(transport: BoxedScpiTransport) -> Self {
        Self { transport }
    }

    pub fn set_timeout_secs(&mut self, secs: u64) -> Result<()> {
        self.transport.set_timeout_secs(secs)
    }

    pub fn identify(&mut self) -> Result<String> {
        self.transport.query_line("*IDN?")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::ScpiTransport;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct MockTransport {
        writes: Arc<Mutex<Vec<String>>>,
        responses: Arc<Mutex<VecDeque<String>>>,
    }

    impl ScpiTransport for MockTransport {
        fn write_line(&mut self, command: &str) -> Result<()> {
            self.writes.lock().unwrap().push(command.to_string());
            Ok(())
        }

        fn query_line(&mut self, command: &str) -> Result<String> {
            self.writes.lock().unwrap().push(command.to_string());
            Ok(self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_default())
        }
    }

    #[test]
    fn identify_queries_idn() {
        let transport = MockTransport::default();
        transport
            .responses
            .lock()
            .unwrap()
            .push_back("KEITHLEY INSTRUMENTS INC.,MODEL 2010,serial,firmware".to_string());
        let writes = Arc::clone(&transport.writes);
        let mut device = Keithley2010::new(Box::new(transport));

        assert_eq!(
            device.identify().unwrap(),
            "KEITHLEY INSTRUMENTS INC.,MODEL 2010,serial,firmware"
        );
        assert_eq!(*writes.lock().unwrap(), vec!["*IDN?".to_string()]);
    }
}
