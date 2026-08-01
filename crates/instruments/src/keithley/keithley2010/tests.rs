use super::*;
use crate::Result;
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
