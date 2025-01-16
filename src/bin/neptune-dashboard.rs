use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process;

use clap::Parser;
use dashboard_src::dashboard_app::DashboardApp;
use neptune_cash::config_models::data_directory::DataDirectory;
use neptune_cash::rpc_auth;
use neptune_cash::rpc_server::RPCClient;
use tarpc::client;
use tarpc::context;
use tarpc::tokio_serde::formats::Json;

pub mod dashboard_src;

#[derive(Debug, Parser, Clone)]
#[clap(name = "neptune-dashboard", about = "Terminal user interface")]
pub struct Config {
    /// Sets the server address to connect to.
    #[clap(long, default_value = "9799", value_name = "PORT")]
    port: u16,

    /// neptune-core data directory containing wallet and blockchain state
    #[clap(long)]
    data_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() {
    // Create connection to RPC server
    let args: Config = Config::parse();
    let server_socket = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), args.port);
    let transport = tarpc::serde_transport::tcp::connect(server_socket, Json::default).await;
    let transport = match transport {
        Ok(transp) => transp,
        Err(err) => {
            eprintln!("{err}");
            eprintln!(
                "Connection to neptune-core failed. Is a node running? Or is the client still \
                starting up?"
            );
            process::exit(1);
        }
    };
    let client = RPCClient::new(client::Config::default(), transport).spawn();

    // note: the network RPC API does not require any auth, to support this usage.

    // Read what network the client is running and ensure that client is up and running
    let network = match client.network(context::current()).await {
        Ok(Ok(nw)) => nw,
        Ok(Err(err)) => {
            eprintln!("{err}");
            eprintln!(
                "Could not ping neptune-core. Do configurations match? Or is the client still \
                starting up?"
            );
            process::exit(1);
        }
        Err(err) => {
            eprintln!("{err}");
            eprintln!(
                "Could not ping neptune-core. Do configurations match? Or is the client still \
                starting up?"
            );
            process::exit(1);
        }
    };

    let data_directory =
        DataDirectory::get(args.data_dir.clone(), network).expect("should find data directory");
    let token: rpc_auth::Token = match rpc_auth::Cookie::try_load(&data_directory) {
        Ok(t) => t,
        Err(e) => panic!(
            "Unable to load RPC authentication token. error = {}",
            e.to_string()
        ),
    }
    .into();

    let listen_addr_for_peers = match client
        .own_listen_address_for_peers(context::current(), token)
        .await
    {
        Ok(Ok(la)) => la,
        Ok(Err(err)) => {
            eprintln!("{err}");
            eprintln!("Could not get listen address from client.");
            process::exit(1);
        }
        Err(err) => {
            eprintln!("{err}");
            eprintln!("Could not get listen address from client.");
            process::exit(1);
        }
    };

    // run app until quit
    let res = DashboardApp::run(client, network, token, listen_addr_for_peers).await;

    match res {
        Err(err) => {
            eprintln!("{:?}", err);
            process::exit(1);
        }
        Ok(output) => {
            print!("{}", output);
        }
    }
}
