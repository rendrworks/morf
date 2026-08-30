impl Runtime {
    /// Takes a successful native authentication request to release a session lock.
    pub fn take_session_unlock_request(&mut self) -> bool {
        std::mem::take(&mut self.reactive.borrow_mut().session_unlock_requested)
    }

    /// Returns registered IPC verb names in lexical order.
    pub fn ipc_verbs(&self) -> Vec<String> {
        let mut verbs = self
            .reactive
            .borrow()
            .ipc_handlers
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        verbs.sort();
        verbs
    }

    /// Calls one registered IPC handler with bounded primitive arguments.
    pub fn call_ipc(&mut self, verb: &str, args: &[IpcValue]) -> Result<Vec<IpcValue>, Error> {
        let handler = self
            .reactive
            .borrow()
            .ipc_handlers
            .get(verb)
            .cloned()
            .ok_or_else(|| Error::Runtime(format!("unknown IPC verb `{verb}`")))?;
        self.lua
            .enter(|ctx| execute_ipc_handler(ctx, &handler, args, self.limits))
            .map_err(Error::Runtime)
    }
}

