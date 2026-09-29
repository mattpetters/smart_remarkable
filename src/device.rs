use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum DeviceModel {
    Remarkable2,
    RemarkablePaperPro,
    RemarkablePaperProMove,
    Unknown,
}

impl DeviceModel {
    pub fn from_string(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "rm2" | "remarkable2" | "remarkable-2" => Ok(DeviceModel::Remarkable2),
            "rmpp" | "remarkable-paper-pro" | "remarkablepaperpro" | "paperpro" => Ok(DeviceModel::RemarkablePaperPro),
            "rmpm" | "rmppm" | "remarkable-paper-pro-move" | "paperpromove" => Ok(DeviceModel::RemarkablePaperProMove),
            _ => Err(anyhow::anyhow!("Invalid device model: {}. Use 'rm2', 'rmpp' or 'rmpm'", s)),
        }
    }

    pub fn detect() -> Self {
        if Path::new("/etc/hwrevision").exists() {
            if let Ok(hwrev) = std::fs::read_to_string("/etc/hwrevision") {
                return Self::from_hardware_revision(&hwrev);
            }
        }

        // Nothing matched :shrug:
        DeviceModel::Unknown
    }

    fn from_hardware_revision(hwrev: &str) -> Self {
        match hwrev.trim() {
            "chiappa 1.0" => Self::RemarkablePaperProMove,
            "ferrari 1.0" => Self::RemarkablePaperPro,
            "reMarkable2 1.0" => Self::Remarkable2,
            _ => Self::Unknown,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            DeviceModel::Remarkable2 => "Remarkable2",
            DeviceModel::RemarkablePaperPro => "RemarkablePaperPro",
            DeviceModel::RemarkablePaperProMove => "RemarkablePaperProMove",
            DeviceModel::Unknown => "Unknown",
        }
    }

    pub fn is_color(self) -> bool {
        matches!(self, Self::RemarkablePaperPro | Self::RemarkablePaperProMove)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_identity_does_not_reuse_paper_pro_geometry() {
        assert_eq!(DeviceModel::from_hardware_revision("chiappa 1.0\n"), DeviceModel::RemarkablePaperProMove);
        assert_eq!(DeviceModel::from_string("rmpm").unwrap(), DeviceModel::RemarkablePaperProMove);
        assert_eq!(DeviceModel::from_hardware_revision("ferrari 1.0"), DeviceModel::RemarkablePaperPro);
        assert_eq!(DeviceModel::from_hardware_revision("chiappa 2.0"), DeviceModel::Unknown);
    }
}
