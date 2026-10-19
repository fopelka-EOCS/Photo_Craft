//! What a model does, which decides the AI tab's section and which tools can use it.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelKind {
    /// Point and box segmentation (SAM family): AI Object Selection.
    Segmentation,
    /// Subject and background matting: selecting people, animals and products.
    Matting,
    /// Prompt-free inpainting: Generative Removal.
    Removal,
    /// Reflection removal brush.
    Reflection,
    /// Prompted diffusion inpainting: the Infill brush and Edit › Generative Fill.
    Inpainting,
    /// Anything imported that doesn't fit the above (kept, listed, never offered to a tool).
    Other,
}

impl ModelKind {
    /// The AI tab's section order.
    pub const ALL: [ModelKind; 6] =
        [ModelKind::Segmentation, ModelKind::Matting, ModelKind::Removal, ModelKind::Reflection, ModelKind::Inpainting, ModelKind::Other];

    /// Section heading in the AI tab.
    pub fn title(self) -> &'static str {
        match self {
            ModelKind::Segmentation => "Object Selection",
            ModelKind::Matting => "Subject & Background",
            ModelKind::Removal => "Generative Removal",
            ModelKind::Reflection => "Reflection Removal",
            ModelKind::Inpainting => "Generative Fill & Infill",
            ModelKind::Other => "Other Models",
        }
    }

    /// One line under the heading: which tools use these models.
    pub fn used_by(self) -> &'static str {
        match self {
            ModelKind::Segmentation => "Used by the AI Object Selection tool.",
            ModelKind::Matting => "Used to select people, animals and products in one click.",
            ModelKind::Removal => "Used by the Generative Removal brush.",
            ModelKind::Reflection => "Used by the Reflection Removal brush.",
            ModelKind::Inpainting => "Used by the Infill brush and Generative Fill (prompts).",
            ModelKind::Other => "Imported models PhotoCraft's tools don't use yet.",
        }
    }

    /// Stable id for preferences, commands and the manifest.
    pub fn id(self) -> &'static str {
        match self {
            ModelKind::Segmentation => "segmentation",
            ModelKind::Matting => "matting",
            ModelKind::Removal => "removal",
            ModelKind::Reflection => "reflection",
            ModelKind::Inpainting => "inpainting",
            ModelKind::Other => "other",
        }
    }

    pub fn from_id(id: &str) -> Option<ModelKind> {
        ModelKind::ALL.into_iter().find(|k| k.id() == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_unknown_ids_are_none() {
        for k in ModelKind::ALL {
            assert_eq!(ModelKind::from_id(k.id()), Some(k));
            assert_eq!(serde_json::to_value(k).ok(), Some(serde_json::json!(k.id())));
        }
        assert_eq!(ModelKind::from_id(""), None);
        assert_eq!(ModelKind::from_id("Segmentation"), None);
    }
}
