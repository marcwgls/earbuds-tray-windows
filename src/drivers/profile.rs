#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseReporting {
    /// Device never reports a case battery.
    Never,
    /// Case battery only arrives while a bud lies in the case (Edifier F2).
    WithBudInCase,
    /// Case battery is part of every status update.
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceProfile {
    pub case: CaseReporting,
}
