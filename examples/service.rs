#[cfg(feature = "swarm")]
mod common;

#[cfg(feature = "swarm")]
use clap::Parser;
#[cfg(feature = "swarm")]
use common::new_docker;

#[cfg(feature = "swarm")]
#[derive(Parser)]
pub struct Opts {
    #[clap(subcommand)]
    subcmd: Cmd,
}

#[cfg(feature = "swarm")]
#[derive(Parser)]
enum Cmd {
    Delete {
        service: String,
    },
    Inspect {
        service: String,
    },
    List {
        #[clap(long)]
        with_status: bool,
    },
    Logs {
        service: String,
        #[clap(long)]
        stdout: bool,
        #[clap(long)]
        stderr: bool,
    },
}

#[cfg(feature = "swarm")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let docker = new_docker()?;
    let opts: Opts = Opts::parse();

    match opts.subcmd {
        Cmd::Delete { service } => {
            if let Err(e) = docker.services().get(&service).delete().await {
                eprintln!("Error: {}", e)
            }
        }
        Cmd::Inspect { service } => {
            match docker.services().get(&service).inspect().await {
                Ok(service) => println!("{:#?}", service),
                Err(e) => eprintln!("Error: {}", e),
            };
        }
        Cmd::List { with_status } => {
            use docker_api::api::ServiceListOpts;

            match docker
                .services()
                .list(&ServiceListOpts::builder().status(with_status).build())
                .await
            {
                Ok(services) => {
                    for s in services {
                        println!("{:#?}", s)
                    }
                }
                Err(e) => eprintln!("Error: {}", e),
            }
        }
        Cmd::Logs {
            service,
            stdout,
            stderr,
        } => {
            use docker_api::api::LogsOpts;
            use futures::StreamExt;

            let logs_stream = docker
                .services()
                .get(&service)
                .logs(&LogsOpts::builder().stdout(stdout).stderr(stderr).build());

            let logs: Vec<_> = logs_stream
                .map(|chunk| match chunk {
                    Ok(chunk) => chunk.to_vec(),
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        vec![]
                    }
                })
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            print!("{}", String::from_utf8_lossy(&logs));
        }
    }

    Ok(())
}

#[cfg(not(feature = "swarm"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}
