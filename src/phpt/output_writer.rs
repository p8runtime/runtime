use crate::runtime::context::HeaderEntry;
use crate::vm::engine::{OutputWriter, VmError};
use std::sync::{Arc, Mutex};

#[derive(Default, Clone)]
pub struct BufferedState {
    pub output: Vec<u8>,
}

#[derive(Clone)]
pub struct BufferedOutputWriter {
    state: Arc<Mutex<BufferedState>>,
}

impl BufferedOutputWriter {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(BufferedState::default())),
        }
    }

    pub fn get_output(&self) -> String {
        let state = self.state.lock().unwrap();
        String::from_utf8_lossy(&state.output).to_string()
    }
}

impl Default for BufferedOutputWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputWriter for BufferedOutputWriter {
    fn write(&mut self, data: &[u8]) -> Result<(), VmError> {
        let mut state = self.state.lock().unwrap();
        state.output.extend_from_slice(data);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), VmError> {
        Ok(())
    }

    fn send_headers(&mut self, _headers: &[HeaderEntry], _status: u16) -> Result<(), VmError> {
        Ok(())
    }

    fn finish(&mut self) -> Result<(), VmError> {
        Ok(())
    }
}
