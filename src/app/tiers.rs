#[cfg(windows)]
pub type Guid = windows::core::GUID;

#[cfg(not(windows))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Guid(pub u128);

#[cfg(windows)]
pub const EDIFIER_SERVICE_UUID: Guid =
    windows::core::GUID::from_u128(0xEDF00000_EDFE_DFED_FEDF_EDFEDFEDFEDF);
#[cfg(windows)]
pub const FAST_PAIR_SERVICE_UUID: Guid =
    windows::core::GUID::from_u128(0xDF21FE2C_2515_4FDB_8886_F12C4D67927C);

/// Samsung Galaxy Buds+ and newer ("SppNew").
#[cfg(windows)]
pub const GALAXY_NEW_SERVICE_UUID: Guid =
    windows::core::GUID::from_u128(0x2E73A4AD_332D_41FC_90E2_16BEF06523F2);
/// Original Galaxy Buds ("SppLegacy").
#[cfg(windows)]
pub const GALAXY_LEGACY_SERVICE_UUID: Guid =
    windows::core::GUID::from_u128(0x00001102_0000_1000_8000_00805F9B34FD);

#[cfg(not(windows))]
pub const EDIFIER_SERVICE_UUID: Guid = Guid(0xEDF00000_EDFE_DFED_FEDF_EDFEDFEDFEDF);
#[cfg(not(windows))]
pub const FAST_PAIR_SERVICE_UUID: Guid = Guid(0xDF21FE2C_2515_4FDB_8886_F12C4D67927C);
#[cfg(not(windows))]
pub const GALAXY_NEW_SERVICE_UUID: Guid = Guid(0x2E73A4AD_332D_41FC_90E2_16BEF06523F2);
#[cfg(not(windows))]
pub const GALAXY_LEGACY_SERVICE_UUID: Guid = Guid(0x00001102_0000_1000_8000_00805F9B34FD);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Vendor {
    Edifier,
    Galaxy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tier {
    Vendor(Vendor),
    FastPair,
    Windows,
}

impl Tier {
    pub fn rank(&self) -> u8 {
        match self {
            Tier::Vendor(Vendor::Edifier) => 1,
            Tier::FastPair => 2,
            // Beta, listen only: below the tested tiers.
            Tier::Vendor(Vendor::Galaxy) => 3,
            Tier::Windows => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TierOverride {
    FastPair,
    Windows,
}

/// The Galaxy RFCOMM service a device publishes. The newer one wins.
pub fn galaxy_service(services: &[Guid]) -> Option<Guid> {
    [GALAXY_NEW_SERVICE_UUID, GALAXY_LEGACY_SERVICE_UUID]
        .into_iter()
        .find(|u| services.contains(u))
}

pub fn tier_for(services: &[Guid], has_hfp_battery: bool) -> Option<Tier> {
    tier_for_with_override(services, has_hfp_battery, None)
}

pub fn tier_for_with_override(
    services: &[Guid],
    has_hfp_battery: bool,
    tier_override: Option<TierOverride>,
) -> Option<Tier> {
    match tier_override {
        Some(TierOverride::FastPair) => {
            if services.contains(&FAST_PAIR_SERVICE_UUID) {
                Some(Tier::FastPair)
            } else if has_hfp_battery {
                Some(Tier::Windows)
            } else {
                None
            }
        }
        Some(TierOverride::Windows) => {
            if has_hfp_battery {
                Some(Tier::Windows)
            } else {
                None
            }
        }
        None => {
            if services.contains(&EDIFIER_SERVICE_UUID) {
                Some(Tier::Vendor(Vendor::Edifier))
            } else if services.contains(&FAST_PAIR_SERVICE_UUID) {
                Some(Tier::FastPair)
            } else if galaxy_service(services).is_some() {
                Some(Tier::Vendor(Vendor::Galaxy))
            } else if has_hfp_battery {
                Some(Tier::Windows)
            } else {
                None
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub name: String,
    pub container: String,
    pub connected: bool,
    pub tier: Tier,
    pub services: Vec<Guid>,
}

pub fn pick<'a>(
    candidates: &'a [Candidate],
    override_filter: Option<&str>,
    last_used: Option<&str>,
) -> Option<&'a Candidate> {
    let filtered: Vec<&'a Candidate> = match override_filter {
        Some(f) => candidates
            .iter()
            .filter(|c| c.name.to_lowercase().contains(&f.to_lowercase()))
            .collect(),
        None => candidates.iter().collect(),
    };

    if filtered.is_empty() {
        return None;
    }

    let has_connected = filtered.iter().any(|c| c.connected);
    let pool: Vec<&'a Candidate> = if has_connected {
        filtered.into_iter().filter(|c| c.connected).collect()
    } else {
        filtered
    };

    let best_rank = pool.iter().map(|c| c.tier.rank()).min()?;
    let best_tier_candidates: Vec<&'a Candidate> = pool
        .into_iter()
        .filter(|c| c.tier.rank() == best_rank)
        .collect();

    if let Some(last) = last_used
        && let Some(matched) = best_tier_candidates
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(last))
    {
        return Some(*matched);
    }

    best_tier_candidates.first().copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tier_for_edifier() {
        let services = vec![EDIFIER_SERVICE_UUID];
        assert_eq!(
            tier_for(&services, true),
            Some(Tier::Vendor(Vendor::Edifier))
        );
        assert_eq!(
            tier_for(&services, false),
            Some(Tier::Vendor(Vendor::Edifier))
        );
    }

    #[test]
    fn test_tier_for_galaxy_new_and_legacy() {
        for uuid in [GALAXY_NEW_SERVICE_UUID, GALAXY_LEGACY_SERVICE_UUID] {
            assert_eq!(tier_for(&[uuid], true), Some(Tier::Vendor(Vendor::Galaxy)));
            assert_eq!(tier_for(&[uuid], false), Some(Tier::Vendor(Vendor::Galaxy)));
        }
    }

    #[test]
    fn test_galaxy_plus_fastpair_keeps_fastpair() {
        let services = [GALAXY_NEW_SERVICE_UUID, FAST_PAIR_SERVICE_UUID];
        assert_eq!(tier_for(&services, true), Some(Tier::FastPair));
    }

    #[test]
    fn test_edifier_beats_galaxy() {
        let services = [GALAXY_NEW_SERVICE_UUID, EDIFIER_SERVICE_UUID];
        assert_eq!(
            tier_for(&services, true),
            Some(Tier::Vendor(Vendor::Edifier))
        );
    }

    #[test]
    fn test_galaxy_with_windows_override() {
        let services = [GALAXY_NEW_SERVICE_UUID];
        assert_eq!(
            tier_for_with_override(&services, true, Some(TierOverride::Windows)),
            Some(Tier::Windows)
        );
        assert_eq!(
            tier_for_with_override(&services, false, Some(TierOverride::Windows)),
            None
        );
    }

    #[test]
    fn test_galaxy_with_fastpair_override_is_skipped() {
        let services = [GALAXY_NEW_SERVICE_UUID];
        assert_eq!(
            tier_for_with_override(&services, true, Some(TierOverride::FastPair)),
            Some(Tier::Windows)
        );
    }

    #[test]
    fn test_standard_spp_alone_is_no_galaxy() {
        #[cfg(windows)]
        let spp = windows::core::GUID::from_u128(0x00001101_0000_1000_8000_00805F9B34FB);
        #[cfg(not(windows))]
        let spp = Guid(0x00001101_0000_1000_8000_00805F9B34FB);
        assert_eq!(tier_for(&[spp], false), None);
        assert_eq!(tier_for(&[spp], true), Some(Tier::Windows));
    }

    #[test]
    fn test_galaxy_service_prefers_new() {
        assert_eq!(
            galaxy_service(&[GALAXY_LEGACY_SERVICE_UUID, GALAXY_NEW_SERVICE_UUID]),
            Some(GALAXY_NEW_SERVICE_UUID)
        );
        assert_eq!(
            galaxy_service(&[GALAXY_LEGACY_SERVICE_UUID]),
            Some(GALAXY_LEGACY_SERVICE_UUID)
        );
        assert_eq!(galaxy_service(&[]), None);
    }

    #[test]
    fn test_tier_for_fastpair() {
        let services = vec![FAST_PAIR_SERVICE_UUID];
        assert_eq!(tier_for(&services, true), Some(Tier::FastPair));
        assert_eq!(tier_for(&services, false), Some(Tier::FastPair));
    }

    #[test]
    fn test_tier_for_windows_hfp() {
        let services = vec![];
        assert_eq!(tier_for(&services, true), Some(Tier::Windows));
    }

    #[test]
    fn test_tier_for_none() {
        let services = vec![];
        assert_eq!(tier_for(&services, false), None);
    }

    #[test]
    fn test_pick_override_filter_case_insensitive() {
        let c1 = Candidate {
            name: "EDIFIER NeoBuds Pro 3".to_string(),
            container: "c1".to_string(),
            connected: true,
            tier: Tier::Vendor(Vendor::Edifier),
            services: vec![],
        };
        let c2 = Candidate {
            name: "Sony WH-1000XM4".to_string(),
            container: "c2".to_string(),
            connected: true,
            tier: Tier::FastPair,
            services: vec![],
        };
        let candidates = vec![c1.clone(), c2.clone()];

        assert_eq!(pick(&candidates, Some("neobuds"), None), Some(&c1));
        assert_eq!(pick(&candidates, Some("SONY"), None), Some(&c2));
    }

    #[test]
    fn test_pick_override_filter_no_match() {
        let c1 = Candidate {
            name: "EDIFIER NeoBuds Pro 3".to_string(),
            container: "c1".to_string(),
            connected: true,
            tier: Tier::Vendor(Vendor::Edifier),
            services: vec![],
        };
        assert_eq!(pick(&[c1], Some("Bose"), None), None);
    }

    #[test]
    fn test_pick_connected_before_unconnected() {
        let c_unconn = Candidate {
            name: "EDIFIER NeoBuds Pro 3".to_string(),
            container: "c1".to_string(),
            connected: false,
            tier: Tier::Vendor(Vendor::Edifier),
            services: vec![],
        };
        let c_conn = Candidate {
            name: "Cheap Headset".to_string(),
            container: "c2".to_string(),
            connected: true,
            tier: Tier::Windows,
            services: vec![],
        };
        assert_eq!(pick(&[c_unconn, c_conn.clone()], None, None), Some(&c_conn));
    }

    #[test]
    fn test_pick_best_tier_wins() {
        let c_win = Candidate {
            name: "Windows Headset".to_string(),
            container: "c1".to_string(),
            connected: true,
            tier: Tier::Windows,
            services: vec![],
        };
        let c_fp = Candidate {
            name: "Pixel Buds Pro".to_string(),
            container: "c2".to_string(),
            connected: true,
            tier: Tier::FastPair,
            services: vec![],
        };
        let c_ed = Candidate {
            name: "EDIFIER NeoBuds Pro 3".to_string(),
            container: "c3".to_string(),
            connected: true,
            tier: Tier::Vendor(Vendor::Edifier),
            services: vec![],
        };
        assert_eq!(
            pick(&[c_win.clone(), c_fp.clone()], None, None),
            Some(&c_fp)
        );
        assert_eq!(pick(&[c_win, c_fp, c_ed.clone()], None, None), Some(&c_ed));
    }

    #[test]
    fn test_pick_galaxy_ranks_below_edifier_and_fastpair() {
        let cand = |name: &str, tier: Tier| Candidate {
            name: name.to_string(),
            container: name.to_string(),
            connected: true,
            tier,
            services: vec![],
        };
        let c_win = cand("Windows Headset", Tier::Windows);
        let c_fp = cand("Pixel Buds Pro", Tier::FastPair);
        let c_ed = cand("EDIFIER NeoBuds Pro 3", Tier::Vendor(Vendor::Edifier));
        let c_gx = cand("Galaxy Buds2 Pro", Tier::Vendor(Vendor::Galaxy));
        // Edifier wins even when the Galaxy Buds were used last.
        assert_eq!(
            pick(
                &[c_gx.clone(), c_ed.clone()],
                None,
                Some("Galaxy Buds2 Pro")
            ),
            Some(&c_ed)
        );
        assert_eq!(pick(&[c_gx.clone(), c_fp.clone()], None, None), Some(&c_fp));
        assert_eq!(pick(&[c_win, c_gx.clone()], None, None), Some(&c_gx));
    }

    #[test]
    fn test_pick_tie_last_used_wins() {
        let c1 = Candidate {
            name: "Pixel Buds Pro".to_string(),
            container: "c1".to_string(),
            connected: true,
            tier: Tier::FastPair,
            services: vec![],
        };
        let c2 = Candidate {
            name: "Sony LinkBuds".to_string(),
            container: "c2".to_string(),
            connected: true,
            tier: Tier::FastPair,
            services: vec![],
        };
        assert_eq!(
            pick(&[c1.clone(), c2.clone()], None, Some("Sony LinkBuds")),
            Some(&c2)
        );
        assert_eq!(
            pick(&[c1.clone(), c2.clone()], None, Some("Pixel Buds Pro")),
            Some(&c1)
        );
    }

    #[test]
    fn test_pick_tie_first_wins() {
        let c1 = Candidate {
            name: "Pixel Buds Pro".to_string(),
            container: "c1".to_string(),
            connected: true,
            tier: Tier::FastPair,
            services: vec![],
        };
        let c2 = Candidate {
            name: "Sony LinkBuds".to_string(),
            container: "c2".to_string(),
            connected: true,
            tier: Tier::FastPair,
            services: vec![],
        };
        assert_eq!(pick(&[c1.clone(), c2], None, None), Some(&c1));
    }

    #[test]
    fn test_pick_unconnected_only_if_nothing_connected() {
        let c1 = Candidate {
            name: "EDIFIER NeoBuds Pro 3".to_string(),
            container: "c1".to_string(),
            connected: false,
            tier: Tier::Vendor(Vendor::Edifier),
            services: vec![],
        };
        let c2 = Candidate {
            name: "Pixel Buds Pro".to_string(),
            container: "c2".to_string(),
            connected: false,
            tier: Tier::FastPair,
            services: vec![],
        };
        // Nothing is connected, so best tier among unconnected is picked
        assert_eq!(pick(&[c1.clone(), c2], None, None), Some(&c1));
    }

    #[test]
    fn test_pick_empty_candidates() {
        assert_eq!(pick(&[], None, None), None);
    }

    #[test]
    fn test_tier_for_with_override_fastpair() {
        // Device with both Edifier and FastPair UUIDs
        let services = [EDIFIER_SERVICE_UUID, FAST_PAIR_SERVICE_UUID];
        assert_eq!(
            tier_for_with_override(&services, true, Some(TierOverride::FastPair)),
            Some(Tier::FastPair)
        );
        // Fallback to None if FastPair not supported
        let edifier_only = [EDIFIER_SERVICE_UUID];
        assert_eq!(
            tier_for_with_override(&edifier_only, false, Some(TierOverride::FastPair)),
            None
        );
        // Fallback to Windows if has_hfp_battery
        assert_eq!(
            tier_for_with_override(&edifier_only, true, Some(TierOverride::FastPair)),
            Some(Tier::Windows)
        );
    }

    #[test]
    fn test_tier_for_with_override_windows() {
        // Device with Edifier and FastPair UUIDs, forcing Windows tier
        let services = [EDIFIER_SERVICE_UUID, FAST_PAIR_SERVICE_UUID];
        assert_eq!(
            tier_for_with_override(&services, true, Some(TierOverride::Windows)),
            Some(Tier::Windows)
        );
        assert_eq!(
            tier_for_with_override(&services, false, Some(TierOverride::Windows)),
            None
        );
    }
}
