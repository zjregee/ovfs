#!/usr/bin/env bash

sudo apt-get update
sudo apt-get install cloud-image-utils
sudo apt-get install qemu qemu-kvm

cloud-localds seed.iso user-data meta-data

wget https://releases.ubuntu.com/20.04/ubuntu-20.04.6-live-server-amd64.iso

sudo mkdir /mnt/ubuntu-iso
sudo mount -o loop ubuntu-20.04.6-live-server-amd64.iso /mnt/ubuntu-iso

truncate -s 20G image.img

qemu-system-x86_64 -enable-kvm -smp 8 -m 16G \
    -drive file=image.img,format=raw,cache=none,if=virtio \
    -drive file=seed.iso,format=raw,cache=none,if=virtio \
    -cdrom ubuntu-20.04.6-live-server-amd64.iso \
    -kernel /mnt/ubuntu-iso/casper/vmlinuz \
    -initrd /mnt/ubuntu-iso/casper/initrd \
    -append "console=ttyS0 autoinstall" \
    -nographic -no-reboot -boot d

sudo umount /mnt/ubuntu-iso
sudo rm -rf /mnt/ubuntu-iso

qemu-system-x86_64 -enable-kvm -smp 8 -m 16G \
    -drive file=image.img,format=raw,cache=none,if=virtio \
    -net user,hostfwd=tcp::2222-:22 -net nic \
    -nographic -boot c
