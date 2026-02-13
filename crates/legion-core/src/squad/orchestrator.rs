//! Squad orchestrator - manages multi-pane tmux layout with independent proxies

use anyhow::Result;

use super::tmux::{
    attach_session, bind_popup_key, configure_pane_borders, create_squad_layout, kill_session,
    send_legion_start, set_pane_title, TmuxLayout,
};

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

    /// Start squad mode: create tmux layout, launch proxy+claude in each pane
    pub fn start(&mut self) -> Result<()> {
        let session_name = format!("legion-squad-{}", std::process::id());

        // Create tmux layout: 65% leader + 35% workers
        let layout = create_squad_layout(&session_name, self.worker_count, 65)?;

        // Configure pane borders for titles
        configure_pane_borders(&session_name)?;

        // Leader pane
        let leader_proxy_port = self.base_port;
        let leader_control_port = self.base_port + 1000;

        // Start legion in leader pane (--no-tmux since we're already in tmux)
        send_legion_start(
            &session_name,
            &layout.leader_pane,
            leader_proxy_port,
            leader_control_port,
        )?;
        set_pane_title(
            &layout.leader_pane,
            &format!("Leader | proxy:{}", leader_proxy_port),
        )?;

        // Bind prefix+p to leader's control port by default
        bind_popup_key(&session_name, leader_control_port)?;

        // Worker panes
        for (i, pane) in layout.worker_panes.iter().enumerate() {
            let worker_proxy_port = self.base_port + 1 + i as u16;
            let worker_control_port = self.base_port + 1000 + 1 + i as u16;

            send_legion_start(
                &session_name,
                pane,
                worker_proxy_port,
                worker_control_port,
            )?;
            set_pane_title(
                pane,
                &format!("Worker {} | proxy:{}", i + 1, worker_proxy_port),
            )?;
        }

        self.layout = Some(layout);

        // Attach to session (blocks until user exits)
        attach_session(&session_name)?;

        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        if let Some(layout) = &self.layout {
            kill_session(&layout.session_name)?;
        }
        Ok(())
    }
}
