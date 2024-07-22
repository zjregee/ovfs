#!/usr/bin/env bash

qemu-system-x86_64 -M pc -cpu host --enable-kvm -smp 8 \
     -m 16G -object memory-backend-file,id=mem,size=16G,mem-path=/dev/shm,share=on -numa node,memdev=mem \
     -chardev socket,id=char0,path=/tmp/vfsd.sock -device vhost-user-fs-pci,queue-size=1024,chardev=char0,tag=myfs \
     -chardev stdio,mux=on,id=mon -mon chardev=mon,mode=readline -device virtio-serial-pci -device virtconsole,chardev=mon -vga none -display none \
     -drive file=image.img,format=raw,cache=none,if=virtio
