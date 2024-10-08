#!/usr/bin/env bash

sudo apt-get update
sudo apt-get install cloud-image-utils qemu qemu-kvm -y

cloud-localds seed.iso user-data meta-data

wget https://releases.ubuntu.com/20.04/ubuntu-20.04.6-live-server-amd64.iso

sudo mkdir /mnt/ubuntu-iso
sudo mount -o loop ubuntu-20.04.6-live-server-amd64.iso /mnt/ubuntu-iso

truncate -s 10G image.img

sudo qemu-system-x86_64 -enable-kvm -smp 2 -m 4G \
    -drive file=image.img,format=raw,cache=none,if=virtio \
    -drive file=seed.iso,format=raw,cache=none,if=virtio \
    -cdrom ubuntu-20.04.6-live-server-amd64.iso \
    -kernel /mnt/ubuntu-iso/casper/vmlinuz \
    -initrd /mnt/ubuntu-iso/casper/initrd \
    -append "console=ttyS0 autoinstall" \
    -nographic -no-reboot -boot d

sudo umount /mnt/ubuntu-iso
sudo rm -rf /mnt/ubuntu-iso

sudo qemu-system-x86_64 --enable-kvm -smp 2 \
    -m 4G -object memory-backend-file,id=mem,size=4G,mem-path=/dev/shm,share=on -numa node,memdev=mem \
    -chardev socket,id=char0,path=/tmp/vfsd.sock -device vhost-user-fs-pci,queue-size=1024,chardev=char0,tag=myfs \
    -drive file=image.img,format=raw,cache=none,if=virtio \
    -net user,hostfwd=tcp::2222-:22 -net nic \
    -nographic -boot c
