use super::args::{McpArgs, McpTarget};
use super::error::{CliError, Result};

pub(crate) fn mcp(a: &McpArgs) -> Result {
    if !a.automation_read_root.is_empty() || !a.automation_write_root.is_empty() {
        let roots = cadcraft_engine::file_access::FileRoots::new(&a.automation_read_root, &a.automation_write_root).map_err(CliError::Roots)?;
        cadcraft_engine::file_access::confine(roots).map_err(CliError::Roots)?;
    }
    let backend: Box<dyn cadcraft_mcp::Backend> = match a.target() {
        McpTarget::Connect { addr, lazy: true, launch } => Box::new(cadcraft_mcp::Remote::lazy(&addr, launch)),
        McpTarget::Connect { addr, .. } => match cadcraft_mcp::Remote::connect(&addr) {
            Ok(r) => Box::new(r),
            Err(source) => return Err(CliError::McpConnect { addr, source }),
        },
        McpTarget::Headless => Box::new(cadcraft_mcp::Headless::default()),
    };
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    cadcraft_mcp::Server::new(backend).serve(stdin.lock(), stdout.lock())?;
    Ok(())
}
