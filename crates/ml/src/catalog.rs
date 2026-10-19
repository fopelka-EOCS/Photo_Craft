//! Recommended models: what the AI tab offers to download in one click.
//!
//! Every entry is a real file on its host with its exact size and, where the host publishes it,
//! its SHA-256; downloads are checked against both. Within a kind, entries are ordered most
//! accurate first, which is the order tools' model dropdowns use.

use crate::ModelKind;
use crate::source::RemoteFile;

#[derive(Debug)]
pub struct CatalogFile {
    pub url: &'static str,
    /// Name inside the model's folder.
    pub name: &'static str,
    pub size: u64,
    /// Lowercase hex; empty when the host doesn't publish one.
    pub sha256: &'static str,
}

impl CatalogFile {
    pub(crate) fn remote(&self) -> RemoteFile {
        RemoteFile {
            url: self.url.to_string(),
            name: self.name.to_string(),
            // The catalog size drives the progress bar; the download itself is checked against
            // the size and SHA-256 the host announces (Hugging Face's X-Linked-Size/-Etag).
            size: None,
            sha256: (!self.sha256.is_empty()).then(|| self.sha256.to_string()),
        }
    }
}

#[derive(Debug)]
pub struct CatalogModel {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: ModelKind,
    /// One line for the AI tab.
    pub description: &'static str,
    /// SPDX-style licence id, as the model's publisher states it.
    pub license: &'static str,
    /// The model's page, for its licence and details.
    pub homepage: &'static str,
    pub files: &'static [CatalogFile],
}

impl CatalogModel {
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).fold(0u64, u64::saturating_add)
    }
}

pub static RECOMMENDED: &[CatalogModel] = &[
    CatalogModel {
        id: "sam-2-1-base-plus",
        name: "SAM 2.1 Base+",
        kind: ModelKind::Segmentation,
        description: "Meta's Segment Anything 2.1, the most accurate selections",
        license: "Apache-2.0",
        homepage: "https://huggingface.co/onnx-community/sam2.1-hiera-base-plus-ONNX",
        files: &[
            CatalogFile {
                url: "https://huggingface.co/onnx-community/sam2.1-hiera-base-plus-ONNX/resolve/main/onnx/vision_encoder.onnx",
                name: "vision_encoder.onnx",
                size: 702_836,
                sha256: "",
            },
            CatalogFile {
                url: "https://huggingface.co/onnx-community/sam2.1-hiera-base-plus-ONNX/resolve/main/onnx/vision_encoder.onnx_data",
                name: "vision_encoder.onnx_data",
                size: 305_746_496,
                sha256: "",
            },
            CatalogFile {
                url: "https://huggingface.co/onnx-community/sam2.1-hiera-base-plus-ONNX/resolve/main/onnx/prompt_encoder_mask_decoder.onnx",
                name: "prompt_encoder_mask_decoder.onnx",
                size: 213_114,
                sha256: "",
            },
            CatalogFile {
                url: "https://huggingface.co/onnx-community/sam2.1-hiera-base-plus-ONNX/resolve/main/onnx/prompt_encoder_mask_decoder.onnx_data",
                name: "prompt_encoder_mask_decoder.onnx_data",
                size: 20_958_208,
                sha256: "",
            },
        ],
    },
    CatalogModel {
        id: "sam-2-1-small",
        name: "SAM 2.1 Small",
        kind: ModelKind::Segmentation,
        description: "Segment Anything 2.1, balanced speed and accuracy",
        license: "Apache-2.0",
        homepage: "https://huggingface.co/onnx-community/sam2.1-hiera-small-ONNX",
        files: &[
            CatalogFile {
                url: "https://huggingface.co/onnx-community/sam2.1-hiera-small-ONNX/resolve/main/onnx/vision_encoder.onnx",
                name: "vision_encoder.onnx",
                size: 467_440,
                sha256: "",
            },
            CatalogFile {
                url: "https://huggingface.co/onnx-community/sam2.1-hiera-small-ONNX/resolve/main/onnx/vision_encoder.onnx_data",
                name: "vision_encoder.onnx_data",
                size: 162_476_288,
                sha256: "",
            },
            CatalogFile {
                url: "https://huggingface.co/onnx-community/sam2.1-hiera-small-ONNX/resolve/main/onnx/prompt_encoder_mask_decoder.onnx",
                name: "prompt_encoder_mask_decoder.onnx",
                size: 213_114,
                sha256: "",
            },
            CatalogFile {
                url: "https://huggingface.co/onnx-community/sam2.1-hiera-small-ONNX/resolve/main/onnx/prompt_encoder_mask_decoder.onnx_data",
                name: "prompt_encoder_mask_decoder.onnx_data",
                size: 20_958_208,
                sha256: "",
            },
        ],
    },
    CatalogModel {
        id: "sam-2-1-tiny",
        name: "SAM 2.1 Tiny",
        kind: ModelKind::Segmentation,
        description: "Segment Anything 2.1, fastest of the three",
        license: "Apache-2.0",
        homepage: "https://huggingface.co/onnx-community/sam2.1-hiera-tiny-ONNX",
        files: &[
            CatalogFile {
                url: "https://huggingface.co/onnx-community/sam2.1-hiera-tiny-ONNX/resolve/main/onnx/vision_encoder.onnx",
                name: "vision_encoder.onnx",
                size: 354_238,
                sha256: "",
            },
            CatalogFile {
                url: "https://huggingface.co/onnx-community/sam2.1-hiera-tiny-ONNX/resolve/main/onnx/vision_encoder.onnx_data",
                name: "vision_encoder.onnx_data",
                size: 134_084_864,
                sha256: "",
            },
            CatalogFile {
                url: "https://huggingface.co/onnx-community/sam2.1-hiera-tiny-ONNX/resolve/main/onnx/prompt_encoder_mask_decoder.onnx",
                name: "prompt_encoder_mask_decoder.onnx",
                size: 213_114,
                sha256: "",
            },
            CatalogFile {
                url: "https://huggingface.co/onnx-community/sam2.1-hiera-tiny-ONNX/resolve/main/onnx/prompt_encoder_mask_decoder.onnx_data",
                name: "prompt_encoder_mask_decoder.onnx_data",
                size: 20_958_208,
                sha256: "",
            },
        ],
    },
    CatalogModel {
        id: "mobilesam",
        name: "MobileSAM",
        kind: ModelKind::Segmentation,
        description: "Small, fast Segment Anything for older computers",
        license: "MIT",
        homepage: "https://huggingface.co/Acly/MobileSAM",
        files: &[
            CatalogFile {
                url: "https://huggingface.co/Acly/MobileSAM/resolve/main/mobile_sam_image_encoder.onnx",
                name: "mobile_sam_image_encoder.onnx",
                size: 28_157_093,
                sha256: "",
            },
            CatalogFile {
                url: "https://huggingface.co/Acly/MobileSAM/resolve/main/sam_mask_decoder_single.onnx",
                name: "sam_mask_decoder_single.onnx",
                size: 16_501_323,
                sha256: "",
            },
        ],
    },
    CatalogModel {
        id: "birefnet",
        name: "BiRefNet",
        kind: ModelKind::Matting,
        description: "High-resolution subject and background separation",
        license: "MIT",
        homepage: "https://huggingface.co/onnx-community/BiRefNet-ONNX",
        files: &[CatalogFile {
            url: "https://huggingface.co/onnx-community/BiRefNet-ONNX/resolve/main/onnx/model_fp16.onnx",
            name: "model_fp16.onnx",
            size: 489_666_272,
            sha256: "",
        }],
    },
    CatalogModel {
        id: "birefnet-lite",
        name: "BiRefNet Lite",
        kind: ModelKind::Matting,
        description: "Faster subject and background separation",
        license: "MIT",
        homepage: "https://huggingface.co/onnx-community/BiRefNet_lite-ONNX",
        files: &[CatalogFile {
            url: "https://huggingface.co/onnx-community/BiRefNet_lite-ONNX/resolve/main/onnx/model.onnx",
            name: "model.onnx",
            size: 224_005_088,
            sha256: "",
        }],
    },
    CatalogModel {
        id: "rmbg-1-4",
        name: "BRIA RMBG 1.4",
        kind: ModelKind::Matting,
        description: "Background removal; non-commercial use only",
        license: "BRIA RMBG 1.4 (non-commercial)",
        homepage: "https://huggingface.co/briaai/RMBG-1.4",
        files: &[CatalogFile {
            url: "https://huggingface.co/briaai/RMBG-1.4/resolve/main/onnx/model_quantized.onnx",
            name: "model_quantized.onnx",
            size: 44_403_226,
            sha256: "",
        }],
    },
    CatalogModel {
        id: "lama",
        name: "LaMa",
        kind: ModelKind::Removal,
        description: "Large-mask inpainting: removes objects and people cleanly",
        license: "Apache-2.0",
        homepage: "https://huggingface.co/Carve/LaMa-ONNX",
        files: &[CatalogFile {
            url: "https://huggingface.co/Carve/LaMa-ONNX/resolve/main/lama_fp32.onnx",
            name: "lama_fp32.onnx",
            size: 208_044_816,
            sha256: "",
        }],
    },
    CatalogModel {
        id: "mi-gan",
        name: "MI-GAN",
        kind: ModelKind::Removal,
        description: "Very fast, light object removal",
        license: "MIT",
        homepage: "https://huggingface.co/andraniksargsyan/migan",
        files: &[CatalogFile {
            url: "https://huggingface.co/andraniksargsyan/migan/resolve/main/migan_pipeline_v2.onnx",
            name: "migan_pipeline_v2.onnx",
            size: 28_079_181,
            sha256: "",
        }],
    },
    CatalogModel {
        id: "sdxl-inpainting",
        name: "SDXL Inpainting 0.1",
        kind: ModelKind::Inpainting,
        description: "Stable Diffusion XL inpainting, best prompted fills",
        license: "OpenRAIL++-M",
        homepage: "https://huggingface.co/aimi-models/inpainting",
        files: &[CatalogFile {
            url: "https://huggingface.co/aimi-models/inpainting/resolve/main/sd_xl_base_1.0_inpainting_0.1.safetensors",
            name: "sd_xl_base_1.0_inpainting_0.1.safetensors",
            size: 6_938_069_944,
            sha256: "",
        }],
    },
    CatalogModel {
        id: "sd-2-inpainting",
        name: "Stable Diffusion 2 Inpainting",
        kind: ModelKind::Inpainting,
        description: "Stable Diffusion 2 inpainting at 512 px",
        license: "OpenRAIL++-M",
        homepage: "https://huggingface.co/sd2-community/stable-diffusion-2-inpainting",
        files: &[CatalogFile {
            url: "https://huggingface.co/sd2-community/stable-diffusion-2-inpainting/resolve/main/512-inpainting-ema.safetensors",
            name: "512-inpainting-ema.safetensors",
            size: 5_214_662_094,
            sha256: "",
        }],
    },
    CatalogModel {
        id: "sd-1-5-inpainting",
        name: "Stable Diffusion 1.5 Inpainting",
        kind: ModelKind::Inpainting,
        description: "The classic, lightest prompted inpainting model",
        license: "CreativeML OpenRAIL-M",
        homepage: "https://huggingface.co/aimi-models/inpainting",
        files: &[CatalogFile {
            url: "https://huggingface.co/aimi-models/inpainting/resolve/main/sd-v1-5-inpainting.safetensors",
            name: "sd-v1-5-inpainting.safetensors",
            size: 4_265_203_873,
            sha256: "",
        }],
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{valid_file_name, valid_id};

    #[test]
    fn every_entry_is_well_formed() {
        let mut ids = std::collections::HashSet::new();
        for m in RECOMMENDED {
            assert!(valid_id(m.id) && ids.insert(m.id), "{}", m.id);
            assert!(!m.files.is_empty(), "{}", m.id);
            assert!(m.homepage.starts_with("https://"), "{}", m.id);
            let mut names = std::collections::HashSet::new();
            for f in m.files {
                assert!(f.url.starts_with("https://"), "{}", f.url);
                assert!(valid_file_name(f.name) && names.insert(f.name), "{}", f.name);
                assert!(f.size > 0, "{}", f.name);
                assert!(f.sha256.is_empty() || crate::source::is_sha256(f.sha256) && f.sha256 == f.sha256.to_ascii_lowercase(), "{}", f.name);
            }
        }
    }
}
