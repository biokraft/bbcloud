use clap::CommandFactory;
use clap_complete::Shell;

/// Writes a completion script for `shell` to stdout. The command factory is
/// supplied by the caller so this module does not depend on the binary's types.
pub fn generate<C: CommandFactory>(shell: Shell) {
    let mut command = C::command();
    let name = command.get_name().to_string();
    clap_complete::generate(shell, &mut command, name, &mut std::io::stdout());
}
