use crate::{clipx, ipc::Client};
use clap::{Args, Subcommand};
use std::{fs, path::Path};

#[derive(Args, Debug)]
pub struct SendArgs {
    #[command(subcommand)]
    cmd: SendSubcmd,
}

#[derive(Subcommand, Debug)]
enum SendSubcmd {
    #[command(about = "Send plain text to a connected device")]
    Text { content: String },
    #[command(about = "Send rich-text content to a connected device")]
    RichText {
        text: String,
        #[arg(long, default_value = "", help = "HTML payload to send with the text")]
        html: String,
    },
    #[command(about = "Send an image file as clipboard content")]
    Image { path: String },
    #[command(about = "Send a file from disk as clipboard content")]
    File {
        path: String,
        #[arg(long, help = "Optional custom file name to send")]
        name: Option<String>,
        #[arg(long, help = "Optional MIME type override")]
        mime_type: Option<String>,
    },
    #[command(about = "Send a file path reference to a connected device")]
    FilePath {
        name: String,
        mime_type: String,
        path: String,
    },
}

impl SendArgs {
    pub fn run(args: SendArgs, ipc: &mut Client) {
        let request = match args.cmd {
            SendSubcmd::Text { content } => clipx::IpcRequest {
                request: Some(clipx::ipc_request::Request::SendText(
                    clipx::SendTextRequest { content },
                )),
            },
            SendSubcmd::RichText { text, html } => clipx::IpcRequest {
                request: Some(clipx::ipc_request::Request::SendRichText(
                    clipx::SendRichTextRequest { text, html },
                )),
            },
            SendSubcmd::Image { path } => {
                let Ok(img) = image::open(&path) else {
                    eprintln!("Unable to open image at {}", path);
                    return;
                };
                let rgba = img.to_rgba8();
                let (width, height) = rgba.dimensions();
                clipx::IpcRequest {
                    request: Some(clipx::ipc_request::Request::SendImage(
                        clipx::SendImageRequest {
                            width,
                            height,
                            rgba: rgba.into_raw(),
                        },
                    )),
                }
            }
            SendSubcmd::File {
                path,
                name,
                mime_type,
            } => {
                let data = match fs::read(&path) {
                    Ok(data) => data,
                    Err(error) => {
                        eprintln!("Failed to read {}: {}", path, error);
                        return;
                    }
                };
                let name = name.unwrap_or_else(|| {
                    Path::new(&path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("upload.bin")
                        .to_string()
                });
                let mime_type = mime_type.unwrap_or_else(|| infer_mime_type(&name));
                clipx::IpcRequest {
                    request: Some(clipx::ipc_request::Request::SendFile(
                        clipx::SendFileRequest {
                            name,
                            mime_type,
                            data,
                        },
                    )),
                }
            }
            SendSubcmd::FilePath {
                name,
                mime_type,
                path,
            } => clipx::IpcRequest {
                request: Some(clipx::ipc_request::Request::SendFilePath(
                    clipx::SendFilePathRequest {
                        name,
                        mime_type,
                        path,
                    },
                )),
            },
        };

        if let Ok(res) = ipc.send(request) {
            if let Some(clipx::ipc_response::Response::Send(response)) = res.response {
                println!("{}", response.message);
            }
        }
    }
}

fn infer_mime_type(name: &str) -> String {
    match Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png".to_string(),
        "jpg" | "jpeg" => "image/jpeg".to_string(),
        "gif" => "image/gif".to_string(),
        "webp" => "image/webp".to_string(),
        "pdf" => "application/pdf".to_string(),
        "txt" => "text/plain".to_string(),
        "html" | "htm" => "text/html".to_string(),
        _ => "application/octet-stream".to_string(),
    }
}
