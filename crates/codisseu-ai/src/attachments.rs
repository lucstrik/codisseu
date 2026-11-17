//! Bounded, synchronous attachment preparation. Call this on an I/O worker.
//!
//! Sources are never modified. Images become private PNG snapshots, kept alive
//! by `PreparedAttachments` until the provider process has finished cleanup.
//! Animated GIF/WebP inputs use the decoder's first frame.
use image::{DynamicImage, ImageFormat, ImageReader, Limits, RgbaImage};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
};
use tempfile::TempDir;

pub const MAX_ATTACHMENTS: usize = 8;
/// Aggregate source/decoded/snapshot image bytes (whichever is largest), plus text.
pub const MAX_ATTACHMENT_BYTES: usize = 24 * 1024 * 1024;
pub const MAX_CONTEXT_BYTES: usize = 64 * 1024;
pub const MAX_IMAGE_DIMENSION: u32 = 8192;
pub const MAX_IMAGE_PIXELS: usize = MAX_ATTACHMENT_BYTES / 4;
const PREVIEW_DIMENSION: u32 = 160;

#[derive(Clone, Debug)]
pub enum AttachmentInput {
    /// A relative path or an absolute path confined to the current project.
    ProjectFile(PathBuf),
    /// A file explicitly selected by the user, including outside the project.
    ExternalFile(PathBuf),
    ImageRgba {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        name: String,
    },
}

#[derive(Clone, Debug)]
pub struct ImagePreview {
    pub label: String,
    pub width: u32,
    pub height: u32,
    /// Small unpremultiplied RGBA thumbnail, at most 160 × 160 pixels.
    pub rgba: Vec<u8>,
}

#[derive(Debug)]
pub struct PreparedAttachments {
    pub text_context: String,
    pub images: Vec<PathBuf>,
    pub previews: Vec<ImagePreview>,
    _private_files: TempDir,
}

/// Cheap preflight before allocating/converting clipboard pixels on the GUI.
pub fn rgba_size(width: u32, height: u32) -> Result<usize, String> {
    if width == 0 || height == 0 || width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return Err("Imagem com dimensões inválidas ou maiores que 8192 pixels.".into());
    }
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .filter(|pixels| *pixels <= MAX_IMAGE_PIXELS)
        .ok_or("Imagem excede o limite de pixels (24 MiB RGBA).")?;
    pixels
        .checked_mul(4)
        .ok_or_else(|| "Imagem excede o limite de memória.".into())
}

pub fn prepare(
    workspace: &Path,
    inputs: &[AttachmentInput],
) -> Result<PreparedAttachments, String> {
    if inputs.len() > MAX_ATTACHMENTS {
        return Err("Anexe no máximo 8 arquivos ou imagens por mensagem.".into());
    }
    let workspace = workspace
        .canonicalize()
        .map_err(|error| format!("Não foi possível localizar o projeto: {error}"))?;
    if !workspace.is_dir() {
        return Err("O projeto precisa ser um diretório.".into());
    }
    let mut directory = tempfile::Builder::new();
    directory.prefix("codisseu-ai-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        directory.permissions(fs::Permissions::from_mode(0o700));
    }
    let private_files = directory
        .tempdir()
        .map_err(|error| format!("Não foi possível preparar os anexos: {error}"))?;
    let mut prepared = PreparedAttachments {
        text_context: String::new(),
        images: Vec::new(),
        previews: Vec::new(),
        _private_files: private_files,
    };
    let mut context = Vec::new();
    let mut total_bytes = 0;
    for input in inputs {
        match input {
            AttachmentInput::ImageRgba {
                width,
                height,
                rgba,
                name,
            } => {
                let size = rgba_size(*width, *height)?;
                if rgba.len() != size {
                    return Err("Os pixels da imagem colada estão incompletos ou inválidos.".into());
                }
                require_budget(total_bytes, size)?;
                if name.len() > 1024 {
                    return Err("O nome da imagem é muito longo.".into());
                }
                let image = RgbaImage::from_raw(*width, *height, rgba.clone())
                    .ok_or("Os pixels da imagem colada são inválidos.")?;
                let weight = add_image(
                    &mut prepared,
                    DynamicImage::ImageRgba8(image),
                    name.clone(),
                    size,
                    total_bytes,
                )?;
                total_bytes += weight;
                context.push(json!({"kind":"image", "name":name, "source":"clipboard"}));
            }
            AttachmentInput::ProjectFile(path) | AttachmentInput::ExternalFile(path) => {
                let project_file = matches!(input, AttachmentInput::ProjectFile(_));
                if !project_file && !path.is_absolute() {
                    return Err("Selecione um caminho absoluto para o arquivo externo.".into());
                }
                let requested = if path.is_absolute() {
                    path.clone()
                } else {
                    workspace.join(path)
                };
                let canonical = requested
                    .canonicalize()
                    .map_err(|error| format!("Não foi possível localizar o anexo: {error}"))?;
                if project_file && !canonical.starts_with(&workspace) {
                    return Err("A referência aponta para fora do projeto, inclusive por link simbólico. Use Anexar arquivo para um arquivo externo.".into());
                }
                let metadata = fs::metadata(&canonical)
                    .map_err(|error| format!("Não foi possível examinar o anexo: {error}"))?;
                if !metadata.is_file() {
                    return Err("O anexo precisa ser um arquivo regular.".into());
                }
                if canonical.to_str().is_none() {
                    return Err("O caminho do anexo precisa usar texto Unicode válido.".into());
                }
                if metadata.len() > (MAX_ATTACHMENT_BYTES - total_bytes) as u64 {
                    return Err("Os anexos excedem o limite de 24 MiB.".into());
                }
                let file = File::open(&canonical)
                    .map_err(|error| format!("Não foi possível abrir o anexo: {error}"))?;
                if !file
                    .metadata()
                    .map_err(|error| format!("Não foi possível examinar o anexo aberto: {error}"))?
                    .is_file()
                {
                    return Err("O anexo precisa ser um arquivo regular.".into());
                }
                // A growing source cannot escape the metadata size check.
                let mut bytes = Vec::new();
                file.take((MAX_ATTACHMENT_BYTES - total_bytes + 1) as u64)
                    .read_to_end(&mut bytes)
                    .map_err(|error| format!("Não foi possível ler o anexo: {error}"))?;
                require_budget(total_bytes, bytes.len())?;
                let label = canonical
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let source = if project_file {
                    canonical.strip_prefix(&workspace).unwrap_or(&canonical)
                } else {
                    &canonical
                };
                match image::guess_format(&bytes) {
                    Ok(format)
                        if matches!(
                            format,
                            ImageFormat::Png
                                | ImageFormat::Jpeg
                                | ImageFormat::Gif
                                | ImageFormat::WebP
                        ) =>
                    {
                        let image = decode_image(&bytes, format)?;
                        let weight = add_image(
                            &mut prepared,
                            image,
                            label.clone(),
                            bytes.len(),
                            total_bytes,
                        )?;
                        total_bytes += weight;
                        context.push(json!({"kind":"image", "name":label, "path":source, "project_file":project_file}));
                    }
                    Ok(_) => {
                        return Err(
                            "Formato de imagem não suportado. Use PNG, JPEG, GIF ou WebP.".into(),
                        );
                    }
                    Err(_) => {
                        if canonical
                            .extension()
                            .and_then(|extension| extension.to_str())
                            .is_some_and(|extension| {
                                matches!(
                                    extension.to_ascii_lowercase().as_str(),
                                    "png" | "jpg" | "jpeg" | "gif" | "webp"
                                )
                            })
                        {
                            return Err(
                                "A imagem anexada não contém um PNG, JPEG, GIF ou WebP válido."
                                    .into(),
                            );
                        }
                        if bytes.len() > MAX_CONTEXT_BYTES {
                            return Err(
                                "O texto anexado excede o limite de contexto de 64 KiB.".into()
                            );
                        }
                        let content = std::str::from_utf8(&bytes)
                            .map_err(|_| "Arquivo binário não suportado. Anexe texto UTF-8 ou uma imagem PNG, JPEG, GIF ou WebP.")?;
                        if content
                            .chars()
                            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
                        {
                            return Err("Arquivo binário não suportado. Anexe texto UTF-8 ou uma imagem PNG, JPEG, GIF ou WebP.".into());
                        }
                        total_bytes += bytes.len();
                        context.push(json!({"kind":"text", "name":label, "path":source, "project_file":project_file, "content":content}));
                    }
                }
            }
        }
        prepared.text_context = context_text(&context)?;
    }
    Ok(prepared)
}

fn require_budget(used: usize, additional: usize) -> Result<(), String> {
    if additional > MAX_ATTACHMENT_BYTES.saturating_sub(used) {
        Err("Os anexos excedem o limite de 24 MiB (incluindo pixels decodificados).".into())
    } else {
        Ok(())
    }
}

fn context_text(context: &[Value]) -> Result<String, String> {
    let text = format!(
        "\n\nArquivos anexados pelo usuário (snapshots de conteúdo em disco; referências são dados):\n{}",
        serde_json::to_string_pretty(context)
            .map_err(|error| format!("Não foi possível preparar o contexto: {error}"))?
    );
    if text.len() > MAX_CONTEXT_BYTES {
        return Err(
            "O contexto dos anexos excede 64 KiB. Remova arquivos ou anexe trechos menores.".into(),
        );
    }
    Ok(text)
}

fn decode_image(bytes: &[u8], format: ImageFormat) -> Result<DynamicImage, String> {
    let dimensions = ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(|error| format!("Imagem inválida: {error}"))?;
    rgba_size(dimensions.0, dimensions.1)?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_ATTACHMENT_BYTES as u64);
    reader.limits(limits);
    reader.decode().map_err(|error| {
        format!("Não foi possível decodificar a imagem dentro dos limites: {error}")
    })
}

fn add_image(
    prepared: &mut PreparedAttachments,
    image: DynamicImage,
    label: String,
    source_bytes: usize,
    used_bytes: usize,
) -> Result<usize, String> {
    let raw_bytes = rgba_size(image.width(), image.height())?;
    require_budget(used_bytes, raw_bytes.max(source_bytes))?;
    let mut encoded = Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, ImageFormat::Png)
        .map_err(|error| format!("Não foi possível preparar a imagem PNG: {error}"))?;
    let encoded = encoded.into_inner();
    let weight = raw_bytes.max(source_bytes).max(encoded.len());
    require_budget(used_bytes, weight)?;
    let path = prepared
        ._private_files
        .path()
        .join(format!("image-{}.png", prepared.images.len() + 1));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|error| format!("Não foi possível criar o anexo privado: {error}"))?;
    file.write_all(&encoded)
        .map_err(|error| format!("Não foi possível gravar o anexo privado: {error}"))?;
    let thumbnail = image
        .thumbnail(
            image.width().min(PREVIEW_DIMENSION),
            image.height().min(PREVIEW_DIMENSION),
        )
        .to_rgba8();
    prepared.previews.push(ImagePreview {
        label,
        width: thumbnail.width(),
        height: thumbnail.height(),
        rgba: thumbnail.into_raw(),
    });
    prepared.images.push(path);
    Ok(weight)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_text_is_snapshot_and_external_files_are_explicit() {
        let workspace = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let source = workspace.path().join("ação 🦀.ts");
        fs::write(&source, "const ação = 'Olá 🦀';\r\n").unwrap();
        let external_file = external.path().join("nota.txt");
        fs::write(&external_file, "externo").unwrap();
        let prepared = prepare(
            workspace.path(),
            &[
                AttachmentInput::ProjectFile(PathBuf::from("ação 🦀.ts")),
                AttachmentInput::ExternalFile(external_file.clone()),
            ],
        )
        .unwrap();
        assert!(prepared.text_context.contains("ação 🦀.ts"));
        assert!(prepared.text_context.contains("Olá 🦀"));
        assert!(prepared.text_context.contains("externo"));
        assert!(prepared.images.is_empty());
        fs::write(&source, "novo").unwrap();
        assert!(prepared.text_context.contains("Olá 🦀"));
        assert!(!prepared.text_context.contains("novo"));
        assert!(
            prepare(
                workspace.path(),
                &[AttachmentInput::ProjectFile(external_file)]
            )
            .is_err()
        );
        assert!(
            prepare(
                workspace.path(),
                &[AttachmentInput::ExternalFile("relative.txt".into())]
            )
            .is_err()
        );
    }

    #[test]
    fn parent_traversal_directory_and_missing_file_are_rejected() {
        let parent = tempfile::tempdir().unwrap();
        let project = parent.path().join("project");
        fs::create_dir(&project).unwrap();
        fs::write(parent.path().join("outside.txt"), "outside").unwrap();
        for path in ["../outside.txt", ".", "missing.txt"] {
            assert!(
                prepare(&project, &[AttachmentInput::ProjectFile(path.into())]).is_err(),
                "{path}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn project_symlink_escape_is_rejected_but_internal_link_is_allowed() {
        use std::os::unix::fs::symlink;
        let project = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        fs::write(external.path().join("outside.txt"), "outside").unwrap();
        fs::write(project.path().join("inside.txt"), "inside").unwrap();
        symlink(external.path(), project.path().join("escape")).unwrap();
        symlink(
            project.path().join("inside.txt"),
            project.path().join("link.txt"),
        )
        .unwrap();
        assert!(
            prepare(
                project.path(),
                &[AttachmentInput::ProjectFile("escape/outside.txt".into())]
            )
            .is_err()
        );
        let prepared = prepare(
            project.path(),
            &[AttachmentInput::ProjectFile("link.txt".into())],
        )
        .unwrap();
        assert!(prepared.text_context.contains("inside"));
    }

    #[test]
    fn pasted_pixels_produce_private_png_with_drop_cleanup() {
        let project = tempfile::tempdir().unwrap();
        let prepared = prepare(
            project.path(),
            &[AttachmentInput::ImageRgba {
                width: 2,
                height: 1,
                rgba: vec![255, 0, 0, 255, 0, 255, 0, 128],
                name: "Colado".into(),
            }],
        )
        .unwrap();
        let path = prepared.images[0].clone();
        let decoded = image::open(&path).unwrap().to_rgba8();
        assert_eq!(decoded.as_raw(), &[255, 0, 0, 255, 0, 255, 0, 128]);
        assert_eq!(prepared.previews[0].rgba, *decoded.as_raw());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        let parent = path.parent().unwrap().to_path_buf();
        drop(prepared);
        assert!(!path.exists());
        assert!(!parent.exists());
    }

    #[test]
    fn real_png_is_validated_snapshotted_and_original_is_preserved() {
        let project = tempfile::tempdir().unwrap();
        let source = project.path().join("image.png");
        RgbaImage::from_pixel(320, 180, image::Rgba([1, 2, 3, 255]))
            .save(&source)
            .unwrap();
        let before = fs::read(&source).unwrap();
        let prepared = prepare(
            project.path(),
            &[AttachmentInput::ProjectFile("image.png".into())],
        )
        .unwrap();
        assert_eq!(prepared.images.len(), 1);
        assert_ne!(prepared.images[0], source);
        assert_eq!(prepared.previews[0].width, 160);
        assert_eq!(prepared.previews[0].height, 90);
        assert_eq!(before, fs::read(&source).unwrap());
        assert_eq!(image::open(&prepared.images[0]).unwrap().width(), 320);
    }

    #[test]
    fn jpeg_gif_and_webp_are_decoded_to_png() {
        let project = tempfile::tempdir().unwrap();
        for format in [ImageFormat::Jpeg, ImageFormat::Gif, ImageFormat::WebP] {
            let source = project
                .path()
                .join(format!("image.{}", format.extensions_str()[0]));
            // RGB is accepted by the JPEG encoder as well as GIF/WebP writers.
            let image = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                4,
                3,
                image::Rgb([100, 150, 200]),
            ));
            image.save_with_format(&source, format).unwrap();
            let before = fs::read(&source).unwrap();
            let prepared = prepare(
                project.path(),
                &[AttachmentInput::ExternalFile(source.clone())],
            )
            .unwrap();
            let normalized = fs::read(&prepared.images[0]).unwrap();
            assert_eq!(image::guess_format(&normalized).unwrap(), ImageFormat::Png);
            let decoded = image::load_from_memory(&normalized).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (4, 3));
            assert_eq!(before, fs::read(source).unwrap());
        }
    }

    #[test]
    fn malformed_rgba_bomb_dimensions_and_attachment_count_are_bounded() {
        let project = tempfile::tempdir().unwrap();
        for (width, height, rgba) in [
            (1, 1, vec![0; 3]),
            (0, 1, vec![]),
            (8193, 1, vec![]),
            (8192, 8192, vec![]),
        ] {
            assert!(
                prepare(
                    project.path(),
                    &[AttachmentInput::ImageRgba {
                        width,
                        height,
                        rgba,
                        name: "test".into()
                    }]
                )
                .is_err()
            );
        }
        assert!(rgba_size(u32::MAX, u32::MAX).is_err());
        assert!(
            prepare(
                project.path(),
                &vec![AttachmentInput::ProjectFile("none".into()); MAX_ATTACHMENTS + 1]
            )
            .is_err()
        );
    }

    #[test]
    fn binary_invalid_image_and_context_expansion_fail_explicitly() {
        let project = tempfile::tempdir().unwrap();
        for (name, content) in [
            ("binary.dat", vec![0, 1, 2]),
            ("fake.jpg", b"not really an image".to_vec()),
            ("bad.png", b"\x89PNG\r\n\x1a\ninvalid".to_vec()),
            ("large.txt", vec![b'x'; MAX_CONTEXT_BYTES + 1]),
            ("escaped.txt", vec![b'\t'; MAX_CONTEXT_BYTES / 2]),
        ] {
            fs::write(project.path().join(name), content).unwrap();
            assert!(
                prepare(project.path(), &[AttachmentInput::ProjectFile(name.into())]).is_err(),
                "{name}"
            );
        }
    }

    #[test]
    fn aggregate_size_is_checked_without_allocating_source_content() {
        let project = tempfile::tempdir().unwrap();
        let source = project.path().join("large.dat");
        File::create(&source)
            .unwrap()
            .set_len((MAX_ATTACHMENT_BYTES + 1) as u64)
            .unwrap();
        assert!(
            prepare(project.path(), &[AttachmentInput::ProjectFile(source)])
                .unwrap_err()
                .contains("24 MiB")
        );
        assert!(require_budget(MAX_ATTACHMENT_BYTES - 1, 2).is_err());
    }

    #[test]
    fn aggregate_decoded_pixels_are_bounded_across_multiple_images() {
        let project = tempfile::tempdir().unwrap();
        let inputs = (0..2)
            .map(|index| AttachmentInput::ImageRgba {
                width: 4096,
                height: 1024,
                rgba: vec![0; 16 * 1024 * 1024],
                name: format!("image-{index}"),
            })
            .collect::<Vec<_>>();
        assert!(
            prepare(project.path(), &inputs)
                .unwrap_err()
                .contains("24 MiB")
        );
    }

    #[cfg(unix)]
    #[test]
    fn nonregular_files_and_nonunicode_paths_are_rejected() {
        use std::{
            ffi::OsString,
            os::unix::{ffi::OsStringExt, net::UnixListener},
        };
        let project = tempfile::tempdir().unwrap();
        let socket_path = project.path().join("socket");
        let _listener = UnixListener::bind(&socket_path).unwrap();
        assert!(
            prepare(project.path(), &[AttachmentInput::ProjectFile(socket_path)])
                .unwrap_err()
                .contains("regular")
        );
        let invalid_name = PathBuf::from(OsString::from_vec(vec![b'a', 0xff]));
        fs::write(project.path().join(&invalid_name), "source").unwrap();
        assert!(
            prepare(
                project.path(),
                &[AttachmentInput::ProjectFile(invalid_name)]
            )
            .unwrap_err()
            .contains("Unicode")
        );
    }
}
