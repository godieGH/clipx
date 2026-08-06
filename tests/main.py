import socket;
import clipx_pb2 as clp;
import os;
import time;
import hashlib;

pub_key = os.urandom(32);

fingerprint = hashlib.sha256(pub_key).digest()

announce = clp.Announce();
announce.fingerprint = fingerprint;
announce.device_name = "Fake Phone";
announce.device_type = clp.ANDROID;
announce.ws_port = 8080

payload = announce.SerializeToString()


sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM);
sock.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1);

print("Fake clipx broadcast running...");

while True:
    sock.sendto(payload, ("255.255.255.255", 9999))
    print("broadcasting...")
    time.sleep(2)