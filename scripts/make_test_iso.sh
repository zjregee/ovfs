#!/usr/bin/env bash

wget https://releases.ubuntu.com/20.04/ubuntu-20.04.6-live-server-amd64.iso

sudo mkdir /mnt/iso
sudo mount -o loop ubuntu-20.04.6-live-server-amd64.iso /mnt/iso
mkdir /tmp/ubuntu-iso
sudo cp -r /mnt/iso/* /tmp/ubuntu-iso/

sudo chmod -R u+w /tmp/ubuntu-iso
mkdir -p /tmp/ubuntu-iso/nocloud
cp ubuntu_auto_install /tmp/ubuntu-iso/nocloud/user-data

# need test
sudo sed -i -e 's/set gfxpayload=keep/set gfxpayload=keep \
    linux   \/casper\/vmlinuz --- autoinstall ds=nocloud\\;seedfrom=\/cdrom\/nocloud\/ \
    initrd  \/casper\/initrd/' /tmp/ubuntu-iso/boot/grub/grub.cfg

cd /tmp/ubuntu-iso
sudo genisoimage -quiet -D -r -V "Ubuntu 20.04 Unattended" -J -l \
    -b isolinux/isolinux.bin -c isolinux/boot.cat -no-emul-boot \
    -boot-load-size 4 -boot-info-table -o ../ubuntu-20.04-unattended.iso .

sudo umount /mnt/iso
sudo rm -rf /mnt/iso
sudo rm -rf /tmp/ubuntu-iso
