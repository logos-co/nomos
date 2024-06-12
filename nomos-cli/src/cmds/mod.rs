use clap::Subcommand;

// pub mod chat;
// pub mod disseminate;

#[derive(Debug, Subcommand)]
pub enum Command {
    // /// Send a blob to the network and collect attestations to create a DA proof
    // Disseminate(disseminate::Disseminate),
    // /// (Almost) Instant messaging protocol on top of the Nomos network
    // Chat(chat::NomosChat),
}

impl Command {
    pub fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        // match self {
        //     Command::Disseminate(cmd) => cmd.run(),
        //     Command::Chat(cmd) => cmd.run(),
        // }
        Ok(())
    }
}
