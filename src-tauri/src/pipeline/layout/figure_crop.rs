use super::media::parse_markdown_image;
use image::GenericImageView;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

const MIN_CLUSTER_IMAGES: usize = 2;
const MAX_CLUSTER_BLOCK_SPAN: usize = 24;
const CROP_MARGIN: i32 = 24;

pub(super) fn replace_split_figure_clusters_with_crops(
    markdown: &str,
    markdown_images: &HashMap<String, String>,
    artifact_root: &Path,
) -> String {
    let page_index = build_page_index(markdown_images);
    if page_index.is_empty() {
        return markdown.to_string();
    }

    let blocks = split_markdown_blocks(markdown);
    let mut output = Vec::with_capacity(blocks.len());
    let mut index = 0usize;
    let mut cluster_index = 1usize;

    while index < blocks.len() {
        if let Some(cluster) = collect_cluster(&blocks, index, markdown_images, &page_index) {
            if let Some(replacement) =
                materialize_cluster_crop(&cluster, cluster_index, artifact_root)
            {
                output.push(replacement);
                index = cluster.end_block + 1;
                cluster_index += 1;
                continue;
            }
        }
        output.push(blocks[index].clone());
        index += 1;
    }

    output.join("\n\n")
}

#[derive(Clone, Debug)]
struct SplitImage {
    bbox: BBox,
    page: usize,
}

#[derive(Clone, Copy, Debug)]
struct BBox {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[derive(Debug)]
struct FigureCluster {
    end_block: usize,
    images: Vec<SplitImage>,
}

fn build_page_index(markdown_images: &HashMap<String, String>) -> HashMap<String, usize> {
    markdown_images
        .iter()
        .filter_map(|(path, value)| {
            let page = parse_ocr_input_page(path)?;
            let uuid = extract_ocr_uuid(value)?;
            Some((uuid, page))
        })
        .collect()
}

fn collect_cluster(
    blocks: &[String],
    start: usize,
    markdown_images: &HashMap<String, String>,
    page_index: &HashMap<String, usize>,
) -> Option<FigureCluster> {
    let first = split_image_from_block(&blocks[start], markdown_images, page_index)?;
    let mut images = vec![first.clone()];
    let mut end_block = start;

    for offset in 1..MAX_CLUSTER_BLOCK_SPAN {
        let Some(block) = blocks.get(start + offset) else {
            break;
        };
        if let Some(image) = split_image_from_block(block, markdown_images, page_index) {
            if image.page != first.page || !bbox_is_near(&first.bbox, &image.bbox) {
                break;
            }
            images.push(image);
            end_block = start + offset;
            continue;
        }
        if !can_skip_between_split_images(block) {
            break;
        }
    }

    (images.len() >= MIN_CLUSTER_IMAGES).then_some(FigureCluster { end_block, images })
}

fn split_image_from_block(
    block: &str,
    markdown_images: &HashMap<String, String>,
    page_index: &HashMap<String, usize>,
) -> Option<SplitImage> {
    let (_, src) = parse_markdown_image(block)?;
    let bbox = parse_ocr_split_image_bbox(&src)?;
    let uuid = markdown_images
        .get(&src)
        .and_then(|value| extract_ocr_uuid(value))?;
    let page = *page_index.get(&uuid)?;
    Some(SplitImage { bbox, page })
}

fn can_skip_between_split_images(block: &str) -> bool {
    let trimmed = block.trim();
    if trimmed.is_empty() {
        return true;
    }
    if trimmed.starts_with("```") || trimmed.starts_with("<!--") {
        return false;
    }
    if trimmed.chars().count() <= 80 {
        return true;
    }
    trimmed.starts_with('#') && trimmed.chars().count() <= 120
}

fn materialize_cluster_crop(
    cluster: &FigureCluster,
    cluster_index: usize,
    artifact_root: &Path,
) -> Option<String> {
    let source_path = artifact_root
        .join("ocr_input")
        .join(format!("page_{}.jpg", cluster.images.first()?.page));
    let output_relative = format!("imgs/figure_cluster_{cluster_index:03}.jpg");
    let output_path = artifact_root.join(sanitize_relative_path(&output_relative));
    if output_path.exists() {
        return Some(markdown_image(&output_relative));
    }

    let image = image::open(&source_path).ok()?;
    let crop = union_bbox(&cluster.images)?;
    let (width, height) = image.dimensions();
    let crop = crop.expand(CROP_MARGIN, width as i32, height as i32)?;
    let cropped = image.crop_imm(
        crop.left as u32,
        crop.top as u32,
        crop.width() as u32,
        crop.height() as u32,
    );
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    cropped.save(&output_path).ok()?;
    Some(markdown_image(&output_relative))
}

fn markdown_image(path: &str) -> String {
    format!("![PDF figure cluster]({path})")
}

fn union_bbox(images: &[SplitImage]) -> Option<BBox> {
    let first = images.first()?.bbox;
    Some(images.iter().fold(first, |acc, image| BBox {
        left: acc.left.min(image.bbox.left),
        top: acc.top.min(image.bbox.top),
        right: acc.right.max(image.bbox.right),
        bottom: acc.bottom.max(image.bbox.bottom),
    }))
}

fn bbox_is_near(first: &BBox, next: &BBox) -> bool {
    let horizontal_gap = if next.left > first.right {
        next.left - first.right
    } else if first.left > next.right {
        first.left - next.right
    } else {
        0
    };
    let vertical_gap = if next.top > first.bottom {
        next.top - first.bottom
    } else if first.top > next.bottom {
        first.top - next.bottom
    } else {
        0
    };
    horizontal_gap <= 420 && vertical_gap <= 620
}

impl BBox {
    fn expand(self, margin: i32, image_width: i32, image_height: i32) -> Option<Self> {
        let left = (self.left - margin).max(0);
        let top = (self.top - margin).max(0);
        let right = (self.right + margin).min(image_width);
        let bottom = (self.bottom + margin).min(image_height);
        (right > left && bottom > top).then_some(Self {
            left,
            top,
            right,
            bottom,
        })
    }

    fn width(self) -> i32 {
        self.right - self.left
    }

    fn height(self) -> i32 {
        self.bottom - self.top
    }
}

fn parse_ocr_input_page(path: &str) -> Option<usize> {
    let file_name = Path::new(path).file_name()?.to_string_lossy();
    file_name
        .strip_prefix("page_")?
        .strip_suffix(".jpg")?
        .parse()
        .ok()
}

fn parse_ocr_split_image_bbox(path: &str) -> Option<BBox> {
    let file_name = Path::new(path).file_stem()?.to_string_lossy();
    let raw = file_name
        .strip_prefix("img_in_chart_box_")
        .or_else(|| file_name.strip_prefix("img_in_image_box_"))?;
    let values = raw
        .split('_')
        .map(str::parse::<i32>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if values.len() != 4 {
        return None;
    }
    let bbox = BBox {
        left: values[0],
        top: values[1],
        right: values[2],
        bottom: values[3],
    };
    (bbox.right > bbox.left && bbox.bottom > bbox.top).then_some(bbox)
}

fn extract_ocr_uuid(value: &str) -> Option<String> {
    let marker = "/pp-ocr-vl-15//";
    let start = value.find(marker)? + marker.len();
    let rest = &value[start..];
    let end = rest.find('/')?;
    Some(rest[..end].to_string())
}

fn split_markdown_blocks(markdown: &str) -> Vec<String> {
    markdown
        .split("\n\n")
        .map(str::trim)
        .filter(|block| !block.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn sanitize_relative_path(raw: &str) -> PathBuf {
    let mut path = PathBuf::new();
    for component in Path::new(raw).components() {
        if let Component::Normal(value) = component {
            path.push(value);
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_ocr_split_image_bbox() {
        let bbox =
            parse_ocr_split_image_bbox("imgs/img_in_chart_box_177_202_469_485.jpg").expect("bbox");

        assert_eq!(bbox.left, 177);
        assert_eq!(bbox.top, 202);
        assert_eq!(bbox.right, 469);
        assert_eq!(bbox.bottom, 485);
    }

    #[test]
    fn builds_page_index_from_ocr_image_urls() {
        let images = HashMap::from([
            (
                "ocr_input/page_4.jpg".to_string(),
                "https://example/pp-ocr-vl-15//abc/input_img_0.jpg".to_string(),
            ),
            (
                "imgs/img_in_chart_box_1_2_3_4.jpg".to_string(),
                "https://example/pp-ocr-vl-15//abc/markdown_0/imgs/img.jpg".to_string(),
            ),
        ]);

        let index = build_page_index(&images);

        assert_eq!(index.get("abc"), Some(&4));
    }

    #[test]
    fn replaces_same_page_split_images_with_cluster_reference() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-figure-crop-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(root.join("ocr_input")).expect("create input dir");
        let image = image::RgbImage::from_pixel(120, 120, image::Rgb([255, 255, 255]));
        image
            .save(root.join("ocr_input/page_4.jpg"))
            .expect("save page");
        let markdown = concat!(
            "![](imgs/img_in_chart_box_10_10_40_40.jpg)\n\n",
            "a\n\n",
            "![](imgs/img_in_chart_box_50_10_90_40.jpg)\n\n",
            "Caption stays after the figure."
        );
        let images = HashMap::from([
            (
                "ocr_input/page_4.jpg".to_string(),
                "https://example/pp-ocr-vl-15//abc/input_img_0.jpg".to_string(),
            ),
            (
                "imgs/img_in_chart_box_10_10_40_40.jpg".to_string(),
                "https://example/pp-ocr-vl-15//abc/markdown_0/imgs/a.jpg".to_string(),
            ),
            (
                "imgs/img_in_chart_box_50_10_90_40.jpg".to_string(),
                "https://example/pp-ocr-vl-15//abc/markdown_0/imgs/b.jpg".to_string(),
            ),
        ]);

        let output = replace_split_figure_clusters_with_crops(markdown, &images, &root);

        assert!(output.contains("![PDF figure cluster](imgs/figure_cluster_001.jpg)"));
        assert!(!output.contains("img_in_chart_box_10_10_40_40"));
        assert!(output.contains("Caption stays after the figure."));
        assert!(root.join("imgs/figure_cluster_001.jpg").exists());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[ignore = "requires local OCR artifact with materialized page images"]
    fn real_empirical_artifact_generates_figure_cluster_crop() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            ".runtime/artifacts/gate-pdf-empirical-article-001-latest-20260530-singlepdf-001",
        );
        if !root.exists() {
            eprintln!("skip: missing {}", root.display());
            return;
        }
        let markdown = std::fs::read_to_string(root.join("source.md")).expect("source markdown");
        let images = std::fs::read_to_string(root.join("source_images.json"))
            .and_then(|raw| {
                serde_json::from_str::<HashMap<String, String>>(&raw).map_err(std::io::Error::other)
            })
            .expect("source images");

        let output = replace_split_figure_clusters_with_crops(&markdown, &images, &root);
        let crop = root.join("imgs/figure_cluster_001.jpg");

        assert!(output.contains("![PDF figure cluster](imgs/figure_cluster_001.jpg)"));
        assert!(crop.exists(), "missing generated crop {}", crop.display());
        assert!(std::fs::metadata(&crop).expect("crop metadata").len() > 10_000);
    }
}
