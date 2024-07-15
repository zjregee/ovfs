use std::io;
use std::sync::Arc;
use std::sync::RwLock;

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
use vm_memory::GuestMemoryAtomic;
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

const HIPRIO_QUEUE_EVENT: u16 = 0;
const REQ_QUEUE_EVENT: u16 = 1;
const QUEUE_SIZE: usize = 32768;
const REQUEST_QUEUES: usize = 1;
const NUM_QUEUES: usize = REQUEST_QUEUES + 1;

struct VhostUserFsThread {
    mem: Option<GuestMemoryAtomic<GuestMemoryMmap>>,
    vu_req: Option<Backend>,
    event_idx: bool,
    kill_event_fd: EventFd,
}

impl VhostUserFsThread {
    fn new() -> Result<VhostUserFsThread> {
        let event_fd = EventFd::new(libc::EFD_NONBLOCK).map_err(|err| {
            new_unexpected_error("failed to create kill eventfd", Some(err.into()))
        })?;
        Ok(VhostUserFsThread {
            mem: None,
            vu_req: None,
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

    fn process_queue_serial(&self, _vring_state: &mut VringState) -> Result<bool> {
        unimplemented!()
    }
}

pub struct VhostUserFsBackend {
    thread: RwLock<VhostUserFsThread>,
}

impl VhostUserFsBackend {
    pub fn new() -> Result<VhostUserFsBackend> {
        let thread = RwLock::new(VhostUserFsThread::new()?);
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

    fn set_backend_req_fd(&self, vu_req: Backend) {
        self.thread.write().unwrap().vu_req = Some(vu_req);
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
    let fs_backend = Arc::new(VhostUserFsBackend::new().unwrap());
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
