use std::io;
use std::process::exit;
use std::sync::Arc;
use std::sync::RwLock;

use log::error;
use log::info;
use log::warn;
use opendal::services::Fs;
use opendal::Operator;
use vhost::vhost_user::message::VhostUserProtocolFeatures;
use vhost::vhost_user::message::VhostUserVirtioFeatures;
use vhost::vhost_user::Backend;
use vhost::vhost_user::Listener;
use vhost_user_backend::VhostUserBackend;
use vhost_user_backend::VhostUserDaemon;
use vhost_user_backend::VringMutex;
use vhost_user_backend::VringState;
use vhost_user_backend::VringT;
use virtio_bindings::bindings::virtio_config::VIRTIO_F_VERSION_1;
use virtio_bindings::bindings::virtio_ring::VIRTIO_RING_F_EVENT_IDX;
use virtio_bindings::bindings::virtio_ring::VIRTIO_RING_F_INDIRECT_DESC;
use virtio_queue::DescriptorChain;
use virtio_queue::QueueOwnedT;
use vm_memory::GuestAddressSpace;
use vm_memory::GuestMemoryAtomic;
use vm_memory::GuestMemoryLoadGuard;
use vm_memory::GuestMemoryMmap;
use vmm_sys_util::epoll::EventSet;
use vmm_sys_util::eventfd::EventFd;

mod buffer;
mod error;
mod filesystem;
mod filesystem_message;
mod util;

use crate::error::*;
use crate::filesystem::Filesystem;
use crate::util::Reader;
use crate::util::Writer;

const HIPRIO_QUEUE_EVENT: u16 = 0;
const REQ_QUEUE_EVENT: u16 = 1;
const QUEUE_SIZE: usize = 1024;
const REQUEST_QUEUES: usize = 1;
const NUM_QUEUES: usize = REQUEST_QUEUES + 1;

struct VhostUserFsThread {
    mem: Option<GuestMemoryAtomic<GuestMemoryMmap>>,
    server: Filesystem,
    vu_req: Option<Backend>,
    event_idx: bool,
    kill_event_fd: EventFd,
}

impl VhostUserFsThread {
    fn new(fs: Filesystem) -> Result<VhostUserFsThread> {
        let event_fd = EventFd::new(libc::EFD_NONBLOCK).map_err(|err| {
            new_unexpected_error("failed to create kill eventfd", Some(err.into()))
        })?;
        Ok(VhostUserFsThread {
            mem: None,
            server: fs,
            vu_req: None,
            event_idx: false,
            kill_event_fd: event_fd,
        })
    }

    fn return_descriptor(
        vring_state: &mut VringState,
        head_index: u16,
        event_idx: bool,
        len: usize,
    ) {
        let used_len: u32 = match len.try_into() {
            Ok(l) => l,
            Err(_) => {
                error!("invalid used length, can't return used descritors to the ring");
                exit(1);
            }
        };
        if vring_state.add_used(head_index, used_len).is_err() {
            warn!("couldn't return used descriptors to the ring");
        }
        if event_idx {
            match vring_state.needs_notification() {
                Err(_) => {
                    warn!("couldn't check if queue needs to be notified");
                    vring_state.signal_used_queue().unwrap();
                }
                Ok(needs_notification) => {
                    if needs_notification {
                        vring_state.signal_used_queue().unwrap();
                    }
                }
            }
        } else {
            vring_state.signal_used_queue().unwrap();
        }
    }

    fn process_queue_serial(&self, vring_state: &mut VringState) -> Result<bool> {
        let mut used_any = false;
        let mem = match &self.mem {
            Some(m) => m.memory(),
            None => return Err(new_unexpected_error("no memory configured", None)),
        };
        let avail_chains: Vec<DescriptorChain<GuestMemoryLoadGuard<GuestMemoryMmap>>> = vring_state
            .get_queue_mut()
            .iter(mem.clone())
            .map_err(|_| new_unexpected_error("iterating through the queue failed", None))?
            .collect();
        for chain in avail_chains {
            used_any = true;
            let head_index = chain.head_index();
            let reader = Reader::new(&mem, chain.clone())
                .map_err(|_| new_unexpected_error("creating a queue reader failed", None))
                .unwrap();
            let writer = Writer::new(&mem, chain.clone())
                .map_err(|_| new_unexpected_error("creating a queue writer failed", None))
                .unwrap();
            let len = self
                .server
                .handle_message(reader, writer)
                .map_err(|_| new_unexpected_error("processing a queue writer failed", None))
                .unwrap();
            VhostUserFsThread::return_descriptor(vring_state, head_index, self.event_idx, len);
        }
        Ok(used_any)
    }

    fn handle_event_serial(&self, device_event: u16, vrings: &[VringMutex]) -> Result<()> {
        let mut vring_state = match device_event {
            HIPRIO_QUEUE_EVENT => vrings[0].get_mut(),
            REQ_QUEUE_EVENT => vrings[1].get_mut(),
            _ => return Err(new_unexpected_error("failed to handle unknown event", None)),
        };
        if self.event_idx {
            loop {
                vring_state.disable_notification().unwrap();
                self.process_queue_serial(&mut vring_state)?;
                if !vring_state.enable_notification().unwrap() {
                    break;
                }
            }
        } else {
            self.process_queue_serial(&mut vring_state)?;
        }
        Ok(())
    }
}

struct VhostUserFsBackend {
    thread: RwLock<VhostUserFsThread>,
}

impl VhostUserFsBackend {
    fn new(fs: Filesystem) -> Result<VhostUserFsBackend> {
        let thread = RwLock::new(VhostUserFsThread::new(fs)?);
        Ok(VhostUserFsBackend { thread })
    }
}

impl VhostUserBackend for VhostUserFsBackend {
    type Bitmap = ();
    type Vring = VringMutex;

    fn num_queues(&self) -> usize {
        NUM_QUEUES
    }

    fn max_queue_size(&self) -> usize {
        QUEUE_SIZE
    }

    fn features(&self) -> u64 {
        1 << VIRTIO_F_VERSION_1
            | 1 << VIRTIO_RING_F_INDIRECT_DESC
            | 1 << VIRTIO_RING_F_EVENT_IDX
            | VhostUserVirtioFeatures::PROTOCOL_FEATURES.bits()
    }

    fn protocol_features(&self) -> VhostUserProtocolFeatures {
        let protocol_features = VhostUserProtocolFeatures::MQ
            | VhostUserProtocolFeatures::BACKEND_REQ
            | VhostUserProtocolFeatures::BACKEND_SEND_FD
            | VhostUserProtocolFeatures::REPLY_ACK
            | VhostUserProtocolFeatures::CONFIGURE_MEM_SLOTS;
        protocol_features
    }

    fn set_event_idx(&self, enabled: bool) {
        self.thread.write().unwrap().event_idx = enabled;
    }

    fn update_memory(&self, mem: GuestMemoryAtomic<GuestMemoryMmap>) -> io::Result<()> {
        self.thread.write().unwrap().mem = Some(mem);
        Ok(())
    }

    fn handle_event(
        &self,
        device_event: u16,
        evset: EventSet,
        vrings: &[VringMutex],
        _thread_id: usize,
    ) -> io::Result<()> {
        if evset != EventSet::IN {
            return Err(new_unexpected_error(
                "failed to handle event other than input event",
                None,
            )
            .into());
        }
        let thread = self.thread.read().unwrap();
        thread
            .handle_event_serial(device_event, vrings)
            .map_err(|err| err.into())
    }

    fn exit_event(&self, _thread_index: usize) -> Option<EventFd> {
        Some(
            self.thread
                .read()
                .unwrap()
                .kill_event_fd
                .try_clone()
                .unwrap(),
        )
    }

    fn set_backend_req_fd(&self, vu_req: Backend) {
        self.thread.write().unwrap().vu_req = Some(vu_req);
    }
}

fn main() {
    env_logger::init();

    let socket = "/tmp/vfsd.sock";
    let share_path = "/home/zjregee/Code/virtio/ovfs/share";
    let mut builder = Fs::default();
    builder.root(share_path);
    let operator = Operator::new(builder)
        .expect("failed to build operator")
        .finish();

    let listener = Listener::new(socket, true).unwrap();
    let fs = Filesystem::new(operator);
    let fs_backend = Arc::new(VhostUserFsBackend::new(fs).unwrap());

    let mut daemon = VhostUserDaemon::new(
        String::from("ovfs-backend"),
        fs_backend.clone(),
        GuestMemoryAtomic::new(GuestMemoryMmap::new()),
    )
    .unwrap();

    if let Err(e) = daemon.start(listener) {
        error!("[Main] failed to start daemon: {:?}", e);
        exit(1);
    }
    info!("[Main] daemon started");

    if let Err(e) = daemon.wait() {
        error!("[Main] failed to wait for daemon: {:?}", e);
    }
    info!("[Main] daemon shutdown");

    let kill_event_fd = fs_backend
        .thread
        .read()
        .unwrap()
        .kill_event_fd
        .try_clone()
        .unwrap();
    if let Err(e) = kill_event_fd.write(1) {
        error!("[Main] failed to shutdown worker thread: {:?}", e);
    }
    info!("[Main] worker thread shutdown");
}
