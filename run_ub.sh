echo "2048 4096 8192"   > /proc/sys/net/ipv4/tcp_wmem
echo "8192 16384 32768" > /proc/sys/net/ipv4/tcp_rmem
echo "4096 131072 262144" > /proc/sys/net/ipv4/tcp_mem

sysctl -w net.core.somaxconn=65535
sysctl -w net.ipv4.tcp_max_syn_backlog=65535

sysctl -w net.ipv4.tcp_fastopen=3
sysctl -w net.ipv4.tcp_tw_reuse=1
sysctl -w net.ipv4.tcp_fin_timeout=10

ulimit -n 65535
ulimit -s unlimited
tachyon --ubdma