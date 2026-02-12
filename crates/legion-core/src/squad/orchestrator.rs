use anyhow::Result;
use super::tmux::{create_squad_layout, send_command_to_pane, attach_session, TmuxLayout};

pub struct SquadOrchestrator {
    layout: Option<TmuxLayout>,
    worker_count: u32,
    base_port: u16,
}

impl SquadOrchestrator {
    pub fn new(worker_count: u32, base_port: u16) -> Self {
        Self {
            layout: None,
            worker_count,
            base_port,
        }
    }

    pub fn start(&mut self) -> Result<()> {
        let session_name = format!("legion-squad-{}", std::process::id());

        // Create tmux layout
        let layout = create_squad_layout(&session_name, self.worker_count)?;

        // Start leader
        let leader_port = self.base_port;
        let leader_cmd = format!("legion start --role leader --port {}", leader_port);
        send_command_to_pane(&session_name, &layout.leader_pane, &leader_cmd)?;

        // Start workers
        for (i, pane) in layout.worker_panes.iter().enumerate() {
            let worker_port = self.base_port + 1 + i as u16;
            let worker_cmd = format!(
                "legion start --role worker --id worker-{} --port {}",
                i + 1,
                worker_port
            );
            send_command_to_pane(&session_name, pane, &worker_cmd)?;
        }

        self.layout = Some(layout);

        // Attach to session
        attach_session(&session_name)?;

        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        if let Some(layout) = &self.layout {
            super::tmux::kill_session(&layout.session_name)?;
        }
        Ok(())
    }
}
