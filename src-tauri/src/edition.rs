//! Where the Full and Lite editions differ (ADR 0007). Ask here rather than testing a Cargo feature.
use serde::Serialize;

// Cargo features only add, so nothing else stops a Lite build from also compiling the CLI and MCP server.
#[cfg(all(feature = "lite", feature = "automation"))]
compile_error!("The Lite edition has no Automation scans: build it with `--no-default-features --features lite`.");

/// The answers the frontend renders by, so it never has to know which edition it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Edition {
    /// The name the app shows for itself.
    pub product_name: &'static str,
    /// Whether the `scan` and `mcp` subcommands exist.
    pub automation_scans: bool,
    /// Whether every Scan target must be an Allowed location or lie inside one.
    pub scan_targets_require_allowed_location: bool,
}

pub const CURRENT: Edition = Edition {
    product_name: if cfg!(feature = "lite") {
        "Hahakigami Lite"
    } else {
        "Hahakigami"
    },
    automation_scans: cfg!(feature = "automation"),
    scan_targets_require_allowed_location: cfg!(feature = "lite"),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "automation")]
    #[test]
    fn the_full_edition_has_automation_scans_and_scans_anywhere() {
        assert_eq!(
            CURRENT,
            Edition {
                product_name: "Hahakigami",
                automation_scans: true,
                scan_targets_require_allowed_location: false,
            }
        );
    }

    #[cfg(feature = "lite")]
    #[test]
    fn the_lite_edition_has_no_automation_scans_and_needs_allowed_locations() {
        assert_eq!(
            CURRENT,
            Edition {
                product_name: "Hahakigami Lite",
                automation_scans: false,
                scan_targets_require_allowed_location: true,
            }
        );
    }

    #[test]
    fn the_frontend_reads_camel_case_answers() {
        assert_eq!(
            serde_json::to_value(CURRENT).unwrap(),
            serde_json::json!({
                "productName": CURRENT.product_name,
                "automationScans": CURRENT.automation_scans,
                "scanTargetsRequireAllowedLocation": CURRENT.scan_targets_require_allowed_location,
            })
        );
    }
}
