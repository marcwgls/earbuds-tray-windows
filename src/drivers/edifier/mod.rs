pub mod packet;
pub mod query;
pub mod transport;

use super::profile::{CaseReporting, DeviceProfile};

pub const PROFILE: DeviceProfile = DeviceProfile {
    case: CaseReporting::WithBudInCase,
};
