# From WSL: can the Windows host's listener on <port> be reached on loopback and on the gateway?
gw=$(ip route show default | cut -d' ' -f3)
echo "gateway $gw"
for h in 127.0.0.1 "$gw"; do curl -s -m 3 -o /dev/null -w "$h:$1 -> %{http_code} %{errormsg}\n" "http://$h:$1"; done
