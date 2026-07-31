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
fn wf1946b_uses_only_scpi_transport() {
    let transport = MockTransport::default();
    transport
        .responses
        .lock()
        .unwrap()
        .push_back("NF,WF1946B,serial,firmware".to_string());
    let writes = Arc::clone(&transport.writes);
    let mut device = WF1946B::new(Box::new(transport));

    assert_eq!(device.identify().unwrap(), "NF,WF1946B,serial,firmware");
    device.trigger().unwrap();

    assert_eq!(
        *writes.lock().unwrap(),
        vec!["*IDN?".to_string(), "*TRG".to_string()]
    );
}
