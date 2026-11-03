#!/bin/sh
set -eu
out=${1:?out dir}
mkdir -p "$out"
cd "$out"
ca() {
  openssl ecparam -name prime256v1 -genkey -noout -out "$1.key"
  openssl req -x509 -new -key "$1.key" -sha256 -days 30 -subj "/CN=$1" -out "$1.pem" \
    -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign"
}
leaf() {
  name=$1; host=$2; signer=$3
  openssl ecparam -name prime256v1 -genkey -noout -out "$name.key"
  openssl req -new -key "$name.key" -subj "/CN=$host" -out "$name.csr"
  printf "subjectAltName=DNS:%s\nextendedKeyUsage=serverAuth,clientAuth\nkeyUsage=critical,digitalSignature\nbasicConstraints=CA:FALSE\n" "$host" > "$name.ext"
  openssl x509 -req -in "$name.csr" -CA "$signer.pem" -CAkey "$signer.key" -CAcreateserial -days 30 -sha256 -extfile "$name.ext" -out "$name.crt"
}
ca ca
ca rogue
leaf a jaeger-a.peer.test ca
leaf b jaeger-b.peer.test ca
leaf w jaeger-w.peer.test ca
leaf evil jaeger-a.peer.test rogue
for n in a b w; do mkdir -p "$n"; cp "$n.crt" "$n/tls.crt"; cp "$n.key" "$n/tls.key"; cp ca.pem "$n/ca.pem"; done
chmod -R a+rX "$out"
