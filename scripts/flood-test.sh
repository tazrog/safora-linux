#!/bin/bash
# Nested stress test: installer fed by a fake backend that floods progress lines.
mkdir -p /tmp/fakebin
cat > /tmp/fakebin/sudo <<'EOS'
#!/bin/bash
for i in $(seq 1 ${NLINES:-60000}); do echo "PROGRESS $((20 + i % 50)) Copying the system ($((i%100))%)"; [ -n "$SLOW" ] && sleep 0.5; done
echo "PROGRESS 100 done"; sleep 30
EOS
chmod +x /tmp/fakebin/sudo
cd /src/target/debug
export PATH=/tmp/fakebin:$PWD:$PATH XDG_CONFIG_HOME=/tmp/cfg-flood LIBGL_ALWAYS_SOFTWARE=1 RUST_LOG=sekurowm=debug
mkdir -p $XDG_CONFIG_HOME
SEKURO_INSTALLER_SELFTEST=1 sekurowm --winit --exec sekuro-bar --exec sekuro-installer >/src/wm-flood.log 2>&1 &
WM=$!
sleep ${WAIT:-16}
echo "--- state responds?"; timeout 3 sekuroctl state | head -c 100; echo " rc=$?"
echo "--- cpu"; ps -o pid,pcpu,stat,wchan:20,comm -C sekurowm,sekuro-installer,sekuro-bar
cat /proc/$WM/task/*/stat | awk '{print $2,$3,$14,$15}' | head
sekuroctl quit; wait $WM
tail -5 /src/wm-flood.log
pkill -f sekuro-installer; pkill -f sekuro-bar; true
