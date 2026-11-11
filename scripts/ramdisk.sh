#!/bin/sh
# A 1 GB RAM disk for the tests' sandboxes, until reboot or `diskutil eject DetachedTests`. It holds
# memory up to its size while mounted.
set -e
if [ "$(uname)" != Darwin ]; then echo "ramdisk: macOS only, nothing done"; exit; fi
if [ -d /Volumes/DetachedTests ]; then echo "ramdisk: already mounted"; exit; fi
dev=$(diskutil image attach --noMount ram://2097152 | awk '{print $1; exit}')
diskutil erasevolume APFS DetachedTests "$dev" >/dev/null
# FSEvents' log of every sandbox's files would fill it; it is read at mount.
mkdir -p /Volumes/DetachedTests/.fseventsd && touch /Volumes/DetachedTests/.fseventsd/no_log
# APFS puts the volume in a container disk of its own, not on "$dev".
vol=$(df /Volumes/DetachedTests | awk 'NR==2 {print $1}')
diskutil unmount /Volumes/DetachedTests >/dev/null && diskutil mount "$vol" >/dev/null
echo "ramdisk: /Volumes/DetachedTests"
