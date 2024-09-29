use crate::{CloudServices, Error};

use sd_cloud_schema::{
	cloud_p2p::{authorize_new_device_in_sync_group, CloudP2PALPN, CloudP2PError},
	devices::{self, Device},
	libraries,
	sync::groups::GroupWithDevices,
};
use sd_crypto::{CryptoRng, SeedableRng};

use iroh_base::key::SecretKey as IrohSecretKey;
use iroh_net::{
	discovery::{
		dns::DnsDiscovery, local_swarm_discovery::LocalSwarmDiscovery, pkarr::dht::DhtDiscovery,
		ConcurrentDiscovery,
	},
	relay::{RelayMap, RelayMode, RelayUrl},
	Endpoint, NodeId,
};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tokio::{spawn, sync::oneshot};
use tracing::error;

mod runner;

use runner::Runner;

#[derive(Debug)]
pub struct JoinedLibraryCreateArgs {
	pub pub_id: libraries::PubId,
	pub name: String,
	pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(transparent)]
#[repr(transparent)]
#[specta(rename = "CloudP2PTicket")]
pub struct Ticket(u64);

#[derive(Debug, Serialize, specta::Type)]
#[serde(tag = "kind", content = "data")]
#[specta(rename = "CloudP2PNotifyUser")]
pub enum NotifyUser {
	ReceivedJoinSyncGroupRequest {
		ticket: Ticket,
		asking_device: Device,
		sync_group: GroupWithDevices,
	},
	ReceivedJoinSyncGroupResponse {
		response: JoinSyncGroupResponse,
		sync_group: GroupWithDevices,
	},
	SendingJoinSyncGroupResponseError {
		error: JoinSyncGroupError,
		sync_group: GroupWithDevices,
	},
	TimedOutJoinRequest {
		device: Device,
		succeeded: bool,
	},
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
pub enum JoinSyncGroupError {
	Communication,
	InternalServer,
	Auth,
}

#[derive(Debug, Serialize, specta::Type)]
pub enum JoinSyncGroupResponse {
	Accepted { authorizor_device: Device },
	Failed(CloudP2PError),
	CriticalError,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct BasicLibraryCreationArgs {
	pub id: libraries::PubId,
	pub name: String,
	pub description: Option<String>,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(tag = "kind", content = "data")]
#[specta(rename = "CloudP2PUserResponse")]
pub enum UserResponse {
	AcceptDeviceInSyncGroup {
		ticket: Ticket,
		accepted: Option<BasicLibraryCreationArgs>,
	},
}
#[derive(Debug, Clone)]
pub struct CloudP2P {
	msgs_tx: flume::Sender<runner::Message>,
}

impl CloudP2P {
	pub async fn new(
		current_device_pub_id: devices::PubId,
		cloud_services: &CloudServices,
		mut rng: CryptoRng,
		iroh_secret_key: IrohSecretKey,
		dns_origin_domain: String,
		dns_pkarr_url: Url,
		relay_url: RelayUrl,
	) -> Result<Self, Error> {
		let endpoint = Endpoint::builder()
			.alpns(vec![CloudP2PALPN::LATEST.to_vec()])
			.discovery(Box::new(ConcurrentDiscovery::from_services(vec![
				Box::new(DnsDiscovery::new(dns_origin_domain)),
				Box::new(
					LocalSwarmDiscovery::new(iroh_secret_key.public())
						.map_err(Error::LocalSwarmDiscoveryInit)?,
				),
				Box::new(
					DhtDiscovery::builder()
						.secret_key(iroh_secret_key.clone())
						.pkarr_relay(dns_pkarr_url)
						.build()
						.map_err(Error::DhtDiscoveryInit)?,
				),
			])))
			.secret_key(iroh_secret_key)
			.relay_mode(RelayMode::Custom(RelayMap::from_url(relay_url)))
			.bind()
			.await
			.map_err(Error::CreateCloudP2PEndpoint)?;

		let (msgs_tx, msgs_rx) = flume::bounded(16);

		spawn({
			let runner = Runner::new(current_device_pub_id, cloud_services, endpoint).await?;
			let user_response_rx = cloud_services.user_response_rx.clone();

			async move {
				// All cloned runners share a single state with internal mutability
				while let Err(e) = spawn(runner.clone().run(
					msgs_rx.clone(),
					user_response_rx.clone(),
					CryptoRng::from_seed(rng.generate_fixed()),
				))
				.await
				{
					if e.is_panic() {
						error!("Cloud P2P runner panicked");
					} else {
						break;
					}
				}
			}
		});

		Ok(Self { msgs_tx })
	}

	/// Requests the device with the given connection ID asking for permission to the current device
	/// to join the sync group
	///
	/// # Panics
	/// Will panic if the actor channel is closed, which should never happen
	pub async fn request_join_sync_group(
		&self,
		devices_in_group: Vec<(devices::PubId, NodeId)>,
		req: authorize_new_device_in_sync_group::Request,
		tx: oneshot::Sender<JoinedLibraryCreateArgs>,
	) {
		self.msgs_tx
			.send_async(runner::Message::Request(runner::Request::JoinSyncGroup {
				req,
				devices_in_group,
				tx,
			}))
			.await
			.expect("Channel closed");
	}
}

impl Drop for CloudP2P {
	fn drop(&mut self) {
		self.msgs_tx.send(runner::Message::Stop).ok();
	}
}
