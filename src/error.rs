//! 统一错误类型。

use std::path::PathBuf;

/// 库错误。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 文件/目录读写失败。
    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),

    /// xdb 文件格式不合法。
    #[error("xdb 格式错误：{0}")]
    Xdb(String),

    /// MMDB 文件读取/解码失败。
    #[error("MMDB 错误：{0}")]
    Mmdb(String),

    /// IP 地址文本不合法。
    #[error("IP 地址无效：{0}")]
    InvalidIp(String),

    /// 调用参数/配置不合法。
    #[error("参数错误：{0}")]
    Argument(String),

    /// 组件源访问失败（网络/HTTP/目录格式）。
    #[error("组件源错误：{0}")]
    Store(String),

    /// 组件源中没有请求的数据集。
    #[error("组件源中未找到数据集：{0}")]
    DatasetNotFound(String),

    /// 数据包校验失败（SHA-256/Ed25519/必需文件）。
    #[error("数据校验失败：{0}")]
    Verify(String),

    /// 数据格式/版本暂不支持。
    #[error("不支持的数据：{0}")]
    Unsupported(String),

    /// 下载授权失败（注册/审批/License/票据/配额）。
    #[error("下载授权失败：{0}")]
    Auth(String),

    /// 路径不存在或不是文件。
    #[error("文件不存在：{0}")]
    MissingFile(PathBuf),
}

#[cfg(feature = "mmdb")]
impl From<maxminddb::MaxMindDbError> for Error {
    fn from(value: maxminddb::MaxMindDbError) -> Self {
        Self::Mmdb(value.to_string())
    }
}

/// 库结果。
pub type Result<T> = std::result::Result<T, Error>;
