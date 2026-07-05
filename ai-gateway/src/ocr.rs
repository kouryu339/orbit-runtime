//! 百度手写文字识别 OCR API

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use corework::buns_system;
use corework::error::FrameworkError;
use corework::orchestration::Context;
use corework::system::SystemOperation;

use base64::{engine::general_purpose, Engine as _};
use image::GenericImageView;
use reqwest::Client;

use crate::error::ApiError;
use crate::types::{BBox2D, ImageSize, OcrResult, OcrResultItem};

// ============================================================================
// IO 类型
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallBaiduOcrInput {
    /// 图片本地路径
    pub image_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallBaiduOcrOutput {
    pub result: OcrResult,
}

// ============================================================================
// System
// ============================================================================

#[buns_system(
    "CallBaiduOcr",
    description = "调用百度手写文字识别 OCR API，返回文字和坐标",
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = true
)]
pub struct CallBaiduOcrSystem;

#[async_trait]
impl SystemOperation for CallBaiduOcrSystem {
    type Input = CallBaiduOcrInput;
    type Output = CallBaiduOcrOutput;
    type Error = FrameworkError;

    fn name(&self) -> &str {
        "CallBaiduOcr"
    }

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let result = call_baidu_ocr(&input.image_path)
            .await
            .map_err(|e| FrameworkError::SystemError(e.to_string()))?;
        Ok(CallBaiduOcrOutput { result })
    }
}

// ============================================================================
// 纯函数实现
// ============================================================================

/// 调用百度手写文字识别 API（从环境变量读取凭证）
pub async fn call_baidu_ocr(image_path: &str) -> crate::error::Result<OcrResult> {
    let api_key = std::env::var("BAIDU_OCR_API_KEY")
        .map_err(|_| ApiError::OcrFailed("未设置 BAIDU_OCR_API_KEY 环境变量".into()))?;
    let secret_key = std::env::var("BAIDU_OCR_SECRET_KEY")
        .map_err(|_| ApiError::OcrFailed("未设置 BAIDU_OCR_SECRET_KEY 环境变量".into()))?;

    let client = Client::new();

    // 1. 获取 access_token
    let token_url = format!(
        "https://aip.baidubce.com/oauth/2.0/token?grant_type=client_credentials&client_id={}&client_secret={}",
        api_key, secret_key
    );

    let token_resp: Value = client
        .post(&token_url)
        .send()
        .await
        .map_err(|e| ApiError::OcrFailed(format!("获取百度 access_token 失败: {e}")))?
        .json()
        .await
        .map_err(|e| ApiError::OcrFailed(format!("解析 token 响应失败: {e}")))?;

    let access_token = token_resp["access_token"]
        .as_str()
        .ok_or_else(|| ApiError::OcrFailed("access_token 不存在".into()))?;

    let image_data =
        std::fs::read(image_path).map_err(|e| ApiError::OcrFailed(format!("读取图片失败: {e}")))?;
    let image_base64 = general_purpose::STANDARD.encode(&image_data);

    // 3. 调用手写文字识别
    let ocr_url = format!(
        "https://aip.baidubce.com/rest/2.0/ocr/v1/handwriting?access_token={}",
        access_token
    );

    let ocr_resp: Value = client
        .post(&ocr_url)
        .form(&[("image", image_base64)])
        .send()
        .await
        .map_err(|e| ApiError::OcrFailed(format!("调用百度 OCR 失败: {e}")))?
        .json()
        .await
        .map_err(|e| ApiError::OcrFailed(format!("解析 OCR 响应失败: {e}")))?;

    // 4. 错误检查
    if let Some(error_code) = ocr_resp.get("error_code") {
        return Err(ApiError::OcrFailed(format!(
            "百度 OCR API 错误: {} - {}",
            error_code,
            ocr_resp
                .get("error_msg")
                .and_then(|v| v.as_str())
                .unwrap_or("未知错误")
        )));
    }

    // 5. 解析
    let words_result = ocr_resp["words_result"]
        .as_array()
        .ok_or_else(|| ApiError::OcrFailed("OCR 响应格式错误".into()))?;

    let items: Vec<OcrResultItem> = words_result
        .iter()
        .map(|word| {
            let text = word["words"].as_str().unwrap_or("").to_string();
            let bbox_2d = word.get("location").and_then(|loc| {
                Some(BBox2D {
                    x1: loc["left"].as_u64()? as u32,
                    y1: loc["top"].as_u64()? as u32,
                    x2: (loc["left"].as_u64()? + loc["width"].as_u64()?) as u32,
                    y2: (loc["top"].as_u64()? + loc["height"].as_u64()?) as u32,
                })
            });
            OcrResultItem {
                text,
                bbox_2d,
                confidence: None,
            }
        })
        .collect();

    // 6. 图片尺寸
    let img =
        image::open(image_path).map_err(|e| ApiError::OcrFailed(format!("打开图片失败: {e}")))?;
    let (w, h) = img.dimensions();

    Ok(OcrResult {
        items,
        image_size: ImageSize {
            width: w,
            height: h,
        },
        elapsed_ms: None,
    })
}
