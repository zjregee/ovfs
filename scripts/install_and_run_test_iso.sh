#!/usr/bin/env bash

qemu-img create -f qcow2 ubuntu.qcow2 20G
qemu-system-x86_64 -enable-kvm -smp 8 -m 16G -cdrom /tmp/ubuntu-20.04-unattended.iso -drive file=ubuntu.qcow2,format=qcow2 -boot d -nographic -no-reboot
