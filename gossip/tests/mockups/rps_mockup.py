#!/usr/bin/python3

import argparse
import hexdump
import socket
import struct
import sys

from Crypto.PublicKey import RSA
from rsa import PublicKey

MOCKUP_ADDR = "127.0.0.1"
MOCKUP_PORT = 7101

RPS_QUERY = 540
RPS_PEER = 541

APP_ONION = 560

RET_PEER_ADDR_FLAG = 0 # 0 ~ IPv4; 1 ~ IPv6
RET_PEER_ADDR = "127.0.0.1"
RET_PEER_HOSTKEY_PATH = './peer2.pub'
RET_PEER_PORT = 6001
RET_PEER_ONION_PORT = 6302
RET_PEER_PORTMAP = 1

RET_PEER_ADDR2 = "127.0.0.1"
RET_PEER_HOSTKEY_PATH2 = './peer3.pub'
RET_PEER_PORT2 = 6303
RET_PEER_ONION_PORT2 = 6303

def read_pem_to_der(path):
    f = open(path, 'r')
    key = RSA.importKey(f.read())
    return key.exportKey('DER')

def alternative_read_pem_to_der(path):
    f = open(path, 'r')
    key = PublicKey.load_pkcs1(f.read(), 'PEM')
    return key.save_pkcs1('DER')

def bad_packet(buf, sock):
    print("[-] Unknown or malformed data received:")
    hexdump.hexdump(buf)
    print("[-] Exiting.")
    sock.close()
    sys.exit(-1)

def get_incoming_conn_socket(addr, port):
    print("[+] Listening for incoming connections on"
          +f" {addr}:{port}")
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)

    s.bind((addr, port))
    s.listen()

    conn, addr_info = s.accept()
    s.close()
    print(f"[+] Peer connected from {addr_info}")
    return conn

def build_answer_payload(addr, port, portmap, keypath):
    pkey_der = read_pem_to_der(keypath)
#    pkey_der = alternative_read_pem_to_der(keypath)
    msize = 8 + 4 + len(portmap)*4 + len(pkey_der)
    ip = socket.inet_aton(addr)
    buf = struct.pack(f">HHHBB",
                      msize,
                      RPS_PEER,
                      RET_PEER_PORT,
                      len(portmap),
                      RET_PEER_ADDR_FLAG)

    for app in portmap:
        port = portmap[app]
        buf += struct.pack(">HH", app, port)

    buf += ip
    buf += pkey_der
    return buf

def main():
    host = MOCKUP_ADDR
    port = MOCKUP_PORT

    first_peer_addr = RET_PEER_ADDR
    first_peer_port = RET_PEER_PORT
    first_peer_key = RET_PEER_HOSTKEY_PATH
    first_portmap = {APP_ONION : RET_PEER_ONION_PORT}

    second_peer_addr = RET_PEER_ADDR2
    second_peer_port = RET_PEER_PORT
    second_peer_key = RET_PEER_HOSTKEY_PATH2
    second_portmap = {APP_ONION : RET_PEER_ONION_PORT2}

    # parse commandline arguments
    usage_string = ("Run an RPS mockup server. Supports up to two alternating,"
                    + " configurable RPS_PEER responses")
    cmd = argparse.ArgumentParser(description=usage_string)
    cmd.add_argument("-a", "--address",
                     help="Mockup Server address to use")
    cmd.add_argument("-p", "--port",
                     help="Mockup Server port to use")
    cmd.add_argument("-1a", "--firstpeer",
                     help="Address:Port of the first peer to return")
    cmd.add_argument("-1p", "--firstports",
                     help="Comma-separated list of APP:PORT pairs of the"
                          + " first peer")
    cmd.add_argument("-1k", "--firstkey",
                     help="Path to the PEM pubkey of the first peer")
    cmd.add_argument("-2a", "--secpeer",
                     help="Address:Port of the second peer to return")
    cmd.add_argument("-2p", "--secports",
                     help="Comma-separated list of APP:PORT pairs of the"
                         + " second peer")
    cmd.add_argument("-2k", "--seckey",
                     help="Path to the PEM pubkey of the sec peer")
    args = cmd.parse_args()

    if args.address is not None:
        host = args.address
    if args.port is not None:
        port = int(args.port)


    if args.firstpeer is not None:
        if (args.firstports is None) or (args.firstkey is None):
            print(f"[-] Portmap and keypath for peer need to be provided.")
            return -1

        first_peer_addr, first_peer_port = args.firstpeer.split(':')
        first_peer_port = int(first_peer_port)
        first_peer_key = args.firstkey

        first_portmap.clear()
        pairs = args.firstports.split(",")
        for pair in pairs:
            app, aport = pair.split(":")
            app = int(app)
            aport = int(aport)
            first_portmap[app] = aport

    if args.secpeer is not None:
        if (args.secports is None) or (args.seckey is None):
            print(f"[-] Portmap, keypath and first peer need to be provided"
                  + "for second peer specification.")
            return -1

        second_peer_addr, second_peer_port = args.secpeer.split(':')
        second_peer_port = int(second_peer_port)
        second_peer_key = args.seckey

        second_portmap.clear()
        pairs = args.secports.split(",")
        for pair in pairs:
            app, aport = pair.split(":")
            app = int(app)
            aport = int(aport)
            second_portmap[app] = aport

    # build answer payloads
    print("[+] Constructed default answer payload:")
    first_answer = build_answer_payload(first_peer_addr,
                                        first_peer_port,
                                        first_portmap,
                                        first_peer_key)
    hexdump.hexdump(first_answer)

    if args.secpeer is not None:
        second_answer = build_answer_payload(second_peer_addr,
                                             second_peer_port,
                                             second_portmap,
                                             second_peer_key)
        print("--------")
        hexdump.hexdump(second_answer)

    # open a socket and listen for incoming connections
    conn = get_incoming_conn_socket(host, port)

    second_buf = False
    # sanity check incoming request
    while True:
        inbuf = conn.recv(4)

        if inbuf == b'':
            print('[-] Connection closed.')
            conn.close()
            conn = get_incoming_conn_socket(host, port)
            continue

        try:
            insize, intype = struct.unpack(">HH", inbuf)
        except Exception:
            bad_packet(inbuf, conn)

        if insize != 4 or intype != RPS_QUERY:
            bad_packet(inbuf, conn)

        peerstr = 'second' if (second_buf and args.secpeer is not None) else 'first'
        print(f"[+] Received RPS_QUERY. Answering with {peerstr} peer.")

        # always answer with the constructed payloads alternating on every
        # RPS_QUERY
        if second_buf and args.secpeer is not None:
            conn.send(second_answer)
            second_buf = False
        else:
            conn.send(first_answer)
            second_buf = True


if __name__ == '__main__':
    main()
