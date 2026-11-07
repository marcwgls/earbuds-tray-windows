#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioState {
    Unknown,
    NotFound,
    Connected,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    PauseWorker,
    ResumeWorker,
    SendDisconnect,
    SendConnect,
}

#[derive(Debug)]
pub struct LinkControl {
    audio: AudioState,
    paused: bool,
}

impl Default for LinkControl {
    fn default() -> Self {
        Self::new()
    }
}

impl LinkControl {
    pub fn new() -> Self {
        Self {
            audio: AudioState::Unknown,
            paused: false,
        }
    }

    pub fn audio(&self) -> AudioState {
        self.audio
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn on_audio(&mut self, state: AudioState) -> Vec<Effect> {
        self.audio = state;
        if self.paused && state == AudioState::Connected {
            self.paused = false;
            vec![Effect::ResumeWorker]
        } else {
            vec![]
        }
    }

    pub fn toggle(&mut self) -> Vec<Effect> {
        match self.audio {
            AudioState::Connected => {
                self.paused = true;
                vec![Effect::PauseWorker, Effect::SendDisconnect]
            }
            AudioState::Disconnected => {
                let mut effects = Vec::new();
                if self.paused {
                    self.paused = false;
                    effects.push(Effect::ResumeWorker);
                }
                effects.push(Effect::SendConnect);
                effects
            }
            AudioState::Unknown | AudioState::NotFound => vec![],
        }
    }

    pub fn on_disconnect_failed(&mut self) -> Vec<Effect> {
        if self.paused {
            self.paused = false;
            vec![Effect::ResumeWorker]
        } else {
            vec![]
        }
    }

    pub fn toggle_label(&self) -> (&'static str, bool) {
        match self.audio {
            AudioState::Connected => ("Disconnect", true),
            AudioState::Disconnected => ("Connect", true),
            AudioState::Unknown | AudioState::NotFound => ("Connect", false),
        }
    }
}

pub fn audio_line(state: AudioState) -> &'static str {
    match state {
        AudioState::Connected => "Audio: connected",
        AudioState::Disconnected => "Audio: not connected",
        AudioState::Unknown => "Audio: unknown",
        AudioState::NotFound => "Audio: device not found",
    }
}

pub fn is_bt_audio_filter(device_id: &str) -> bool {
    let lower = device_id.to_ascii_lowercase();
    lower.starts_with(r"{2}.\\?\bth")
}

pub fn pick_container<'a>(endpoints: &'a [(String, String)], filter: &str) -> Option<&'a str> {
    let filter_lower = filter.to_ascii_lowercase();
    endpoints
        .iter()
        .find(|(_, name)| name.to_ascii_lowercase().contains(&filter_lower))
        .map(|(container, _)| container.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state() {
        let link = LinkControl::new();
        assert_eq!(link.audio(), AudioState::Unknown);
        assert!(!link.is_paused());
        assert_eq!(link.toggle_label(), ("Connect", false));
    }

    #[test]
    fn test_on_audio_stores_state() {
        let mut link = LinkControl::new();
        let effects = link.on_audio(AudioState::Connected);
        assert_eq!(link.audio(), AudioState::Connected);
        assert_eq!(effects, vec![]);
    }

    #[test]
    fn test_on_audio_resumes_worker_when_paused_and_connected() {
        let mut link = LinkControl::new();
        link.on_audio(AudioState::Connected);
        link.toggle(); // transitions to paused
        assert!(link.is_paused());

        let effects = link.on_audio(AudioState::Connected);
        assert!(!link.is_paused());
        assert_eq!(effects, vec![Effect::ResumeWorker]);
    }

    #[test]
    fn test_on_audio_when_paused_and_disconnected_remains_paused() {
        let mut link = LinkControl::new();
        link.on_audio(AudioState::Connected);
        link.toggle();
        assert!(link.is_paused());

        let effects = link.on_audio(AudioState::Disconnected);
        assert!(link.is_paused());
        assert_eq!(effects, vec![]);
    }

    #[test]
    fn test_toggle_when_connected() {
        let mut link = LinkControl::new();
        link.on_audio(AudioState::Connected);
        let effects = link.toggle();
        assert!(link.is_paused());
        assert_eq!(effects, vec![Effect::PauseWorker, Effect::SendDisconnect]);
    }

    #[test]
    fn test_toggle_when_disconnected_and_paused() {
        let mut link = LinkControl::new();
        link.on_audio(AudioState::Connected);
        link.toggle(); // now paused
        link.on_audio(AudioState::Disconnected);
        assert!(link.is_paused());

        let effects = link.toggle();
        assert!(!link.is_paused());
        assert_eq!(effects, vec![Effect::ResumeWorker, Effect::SendConnect]);
    }

    #[test]
    fn test_toggle_when_disconnected_and_not_paused() {
        let mut link = LinkControl::new();
        link.on_audio(AudioState::Disconnected);
        assert!(!link.is_paused());

        let effects = link.toggle();
        assert!(!link.is_paused());
        assert_eq!(effects, vec![Effect::SendConnect]);
    }

    #[test]
    fn test_toggle_when_unknown_or_not_found() {
        let mut link = LinkControl::new();
        assert_eq!(link.toggle(), vec![]);

        link.on_audio(AudioState::NotFound);
        assert_eq!(link.toggle(), vec![]);
    }

    #[test]
    fn test_on_disconnect_failed() {
        let mut link = LinkControl::new();
        assert_eq!(link.on_disconnect_failed(), vec![]);

        link.on_audio(AudioState::Connected);
        link.toggle(); // paused
        assert!(link.is_paused());

        let effects = link.on_disconnect_failed();
        assert!(!link.is_paused());
        assert_eq!(effects, vec![Effect::ResumeWorker]);
    }

    #[test]
    fn test_toggle_label() {
        let mut link = LinkControl::new();
        assert_eq!(link.toggle_label(), ("Connect", false));

        link.on_audio(AudioState::Connected);
        assert_eq!(link.toggle_label(), ("Disconnect", true));

        link.on_audio(AudioState::Disconnected);
        assert_eq!(link.toggle_label(), ("Connect", true));

        link.on_audio(AudioState::NotFound);
        assert_eq!(link.toggle_label(), ("Connect", false));

        link.on_audio(AudioState::Unknown);
        assert_eq!(link.toggle_label(), ("Connect", false));
    }

    #[test]
    fn test_audio_line() {
        assert_eq!(audio_line(AudioState::Connected), "Audio: connected");
        assert_eq!(audio_line(AudioState::Disconnected), "Audio: not connected");
        assert_eq!(audio_line(AudioState::Unknown), "Audio: unknown");
        assert_eq!(audio_line(AudioState::NotFound), "Audio: device not found");
    }

    #[test]
    fn test_is_bt_audio_filter() {
        assert!(is_bt_audio_filter(r"{2}.\\?\bthenum#dev_12345678#0000"));
        assert!(is_bt_audio_filter(r"{2}.\\?\BTHHFENUM#bthhf_guid#0000"));
        assert!(!is_bt_audio_filter(
            r"{2}.\\?\hdaudio#func_01&ven_10ec#0000"
        ));
        assert!(!is_bt_audio_filter(r"{2}.\\?\usb#vid_046d&pid_0825#0000"));
        assert!(!is_bt_audio_filter(""));
    }

    #[test]
    fn test_pick_container() {
        let endpoints = vec![
            (
                "{container-realtek}".to_string(),
                "Speakers (Realtek High Definition Audio)".to_string(),
            ),
            (
                "{container-edifier-1234}".to_string(),
                "Kopfhörer (NeoBuds Pro 3)".to_string(),
            ),
            (
                "{container-edifier-1234}".to_string(),
                "Headset (NeoBuds Pro 3 Hands-Free AG Audio)".to_string(),
            ),
        ];

        assert_eq!(
            pick_container(&endpoints, "NeoBuds Pro 3"),
            Some("{container-edifier-1234}")
        );
        assert_eq!(
            pick_container(&endpoints, "neobuds pro 3"),
            Some("{container-edifier-1234}")
        );
        assert_eq!(pick_container(&endpoints, "AirPods"), None);
    }
}
