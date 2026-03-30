//! XTG 共享业务层：配置、轮询、TG、日志广播。

pub mod command_state;
pub mod paths;
pub mod pipeline;
pub mod settings;
pub mod service;
pub mod state;
pub mod tg_commands;
pub mod translate;

pub use command_state::CommandState;
pub use settings::AppSettings;
pub use service::XtgService;
