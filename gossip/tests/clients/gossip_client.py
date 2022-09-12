#!/usr/bin/python3

import argparse
from time import sleep, time
import hexdump
import socket
import struct
import sys

from util import sync_bad_packet, connect_socket, sync_read_message

GOSSIP_ANNOUNCE = 500
GOSSIP_NOTIFY = 501
GOSSIP_NOTIFICATION = 502
GOSSIP_VALIDATION = 503

GOSSIP_ADDR = "127.0.0.1"
GOSSIP_PORT = 7001

DATA_TYPE = 1337
DATA_CONT = b"deadbeef"
DATA_TTL = 4

def send_gossip_announce(s):
    """
    Send a GOSSIP_ANNOUNCE message to socket s

        s:          connected socket to send message on
    """

    # prepare packet payload
    bsize = 4 + 4 + len(DATA_CONT)
    buf = struct.pack(">HHBBH", bsize, GOSSIP_ANNOUNCE, DATA_TTL, 0, DATA_TYPE)
    buf += DATA_CONT

    print(f"[+] Prepared GOSSIP_ANNOUNCE packet:")
    hexdump.hexdump(buf)

    # Send payload
    s.send(buf)
    print(f"[+] Sent GOSSIP_ANNOUNCE ({DATA_TYPE}, {DATA_CONT})")

def send_gossip_notify(s):
    """
    Send a GOSSIP_NOTIFY message to socket s
        s:          connected socket to send message on
    """
    # prepare packet payload
    bsize = 4 + 4
    buf = struct.pack(">HHHH", bsize, GOSSIP_NOTIFY, 0, DATA_TYPE)

    # Send payload
    s.send(buf)
    print(f"[+] Sent GOSSIP_NOTIFY for type {DATA_TYPE}: {buf}; len {len(buf)}).")

def wait_notification(s):
    """
    Wait for a subsequent GOSSIP_NOTIFICATION message

        s:          connected socket to listen on
        returns:    message ID of notification message
    """
    # Try to read notification for our registered DATA_TYPE
    buf = sync_read_message(s)
    msize, mtype, mid, dtype = struct.unpack(">HHHH", buf[:8])
    mdata = buf[8:]

    if mtype != GOSSIP_NOTIFICATION:
        reason = f"Wrong packet type: {mtype}"
        sync_bad_packet(buf, s, reason)

    print(f"[+] Got GOSSIP_NOTIFICATION: mID = {mid}, type = {dtype}, "
           + f"data = {mdata}")

    hexdump.hexdump(buf)
    return mid

def send_gossip_validation(s, mid, valid=True):
    """
    Send a GOSSIP_VALIDATION message to socket s

        s:          connected socket to send message on
        mid:        message ID to validate
        valid:      bool, signifying if message MID was valid
    """
    # prepare packet payload
    bsize = 4 + 4
    buf = struct.pack(">HHHBB", bsize, GOSSIP_VALIDATION, mid, 0, valid)

    # Send payload
    s.send(buf)
    validstr = "Valid" if valid else "Invalid"
    print(f"[+] Sent GOSSIP_VALIDATION: mid = {mid}, {validstr}).")

def main():

    # allow modification of global default values
    global GOSSIP_ADDR, GOSSIP_PORT

    # parse command line arguments
    usage_string = "Run a GOSSIP module API client"
    cmd = argparse.ArgumentParser(description=usage_string)
    cmd.add_argument("-a", "--announce", action="store_true",
                     help="Send a GOSSIP_ANNOUNCE message to the target")
    cmd.add_argument("-n", "--notify", action="store_true",
                     help="Send a GOSSIP_NOTIFY and subsequent VALIDATION")
    cmd.add_argument("-d", "--destination",
                     help="GOSSIP host module IP to connect to")
    cmd.add_argument("-p", "--port",
                     help="GOSSIP host module port to connect to")
    args = cmd.parse_args()

    if args.destination is not None:
        GOSSIP_ADDR = args.destination

    if args.port is not None:
        GOSSIP_PORT = int(args.port)

    if args.announce:
        exmode = "announce"
    elif args.notify:
        exmode = "notify"
    else:
        exmode = "announce"

    # Connect to GOSSIP server
    s = connect_socket(GOSSIP_ADDR, GOSSIP_PORT)
    print("[+] Connected to gossip module:", (GOSSIP_ADDR, GOSSIP_PORT))

    if exmode == "announce":

        # Send notify request, to register a valid message type
        send_gossip_notify(s)
        print("[+] Finished sending notify; sending announce...")
        sleep(2)
        # input("Press anykey to send gossip_announce message...")
        send_gossip_announce(s)
    else:
        send_gossip_notify(s)

        while(True):
            print("[+] Waiting for GOSSIP_NOTIFICATION message...")
            mid = wait_notification(s)

            send_gossip_validation(s, mid, valid=True)

    s.close()

if __name__ == "__main__":
    main()
