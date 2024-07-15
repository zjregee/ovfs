use std::io;
use std::sync::Arc;
use std::sync::RwLock;

use opendal::services::S3;
use opendal::Operator;
use vhost::vhost_user::message::VhostUserProtocolFeatures;
use vhost::vhost_user::message::VhostUserVirtioFeatures;
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

#[allow(dead_code)]
mod descriptor_utils;
mod error;
#[allow(dead_code)]
mod file_traits;
#[allow(dead_code)]
mod filesystem;
mod filesystem_message;
#[allow(dead_code)]
mod filesystem_util;
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
            event_idx: false,
            kill_event_fd: event_fd,
        })
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

    fn return_descriptor(
        vring_state: &mut VringState,
        head_index: u16,
        event_idx: bool,
        len: usize,
    ) {
        let used_len: u32 = match len.try_into() {
            Ok(l) => l,
            Err(_) => panic!("Invalid used length, can't return used descritors to the ring"),
        };
        if vring_state.add_used(head_index, used_len).is_err() {
            println!("Couldn't return used descriptors to the ring");
        }
        if event_idx {
            match vring_state.needs_notification() {
                Err(_) => {
                    println!("Couldn't check if queue needs to be notified");
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
}

pub struct VhostUserFsBackend {
    thread: RwLock<VhostUserFsThread>,
}

impl VhostUserFsBackend {
    pub fn new(fs: Filesystem) -> Result<VhostUserFsBackend> {
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

    fn handle_event(
        &self,
        device_event: u16,
        evset: EventSet,
        vrings: &[Self::Vring],
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
}

fn main() {
    let socket = "/tmp/vfsd.sock";
    let listener = Listener::new(socket, true).unwrap();
    let mut builder = S3::default();
    builder.bucket("test");
    builder.endpoint("http://127.0.0.1:9000");
    builder.access_key_id("minioadmin");
    builder.secret_access_key("minioadmin");
    builder.region("us-east-1");
    let operator = Operator::new(builder)
        .expect("failed to build operator")
        .finish();
    let fs = Filesystem::new(operator);
    let fs_backend = Arc::new(VhostUserFsBackend::new(fs).unwrap());
    let mut daemon = VhostUserDaemon::new(
        String::from("ovfs-backend"),
        fs_backend.clone(),
        GuestMemoryAtomic::new(GuestMemoryMmap::new()),
    )
    .unwrap();
    if let Err(e) = daemon.start(listener) {
        println!("Failed to start daemon: {}", e);
    }
    if let Err(e) = daemon.wait() {
        println!("Failed to wait for daemon: {}", e);
    }
    let kill_event_fd = fs_backend
        .thread
        .read()
        .unwrap()
        .kill_event_fd
        .try_clone()
        .unwrap();
    if let Err(e) = kill_event_fd.write(1) {
        println!("Error shutting down worker thread: {:?}", e)
    }
}
