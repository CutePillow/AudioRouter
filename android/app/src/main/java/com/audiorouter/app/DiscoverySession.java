package com.audiorouter.app;

import java.io.IOException;
import java.net.DatagramPacket;
import java.net.DatagramSocket;
import java.net.InetAddress;
import java.net.InterfaceAddress;
import java.net.NetworkInterface;
import java.net.SocketException;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

final class DiscoverySession implements AutoCloseable {
    static final int DISCOVERY_PORT = 40401;
    private static final String REQUEST = "AUDIOROUTER_DISCOVER_V1";
    private static final String RESPONSE_PREFIX = "AUDIOROUTER_DEVICE_V1|";
    private volatile boolean closed;
    private DatagramSocket socket;

    interface Listener {
        void onDevices(List<Device> devices);
        void onError(String message);
    }

    static final class Device {
        final String name;
        final InetAddress address;
        final int port;
        Device(String name, InetAddress address, int port) {
            this.name = name;
            this.address = address;
            this.port = port;
        }
    }

    void scan(Listener listener) {
        new Thread(() -> {
            Map<String, Device> devices = new LinkedHashMap<>();
            Set<String> localAddresses = localAddresses();
            try (DatagramSocket current = new DatagramSocket()) {
                socket = current;
                current.setBroadcast(true);
                current.setSoTimeout(250);
                byte[] request = REQUEST.getBytes(StandardCharsets.UTF_8);
                sendDiscoveryRequests(current, request);
                long deadline = System.currentTimeMillis() + 1500;
                byte[] buffer = new byte[512];
                while (!closed && System.currentTimeMillis() < deadline) {
                    try {
                        DatagramPacket packet = new DatagramPacket(buffer, buffer.length);
                        current.receive(packet);
                        Device device = parse(packet, localAddresses);
                        if (device != null) {
                            // A device is identified by its LAN address. Different
                            // responders on the same host must not create duplicate cards.
                            devices.put(device.address.getHostAddress(), device);
                        }
                    } catch (java.net.SocketTimeoutException ignored) {
                    }
                }
                listener.onDevices(new ArrayList<>(devices.values()));
            } catch (IOException error) {
                if (!closed) listener.onError("局域网扫描失败");
            } finally {
                socket = null;
            }
        }, "AudioRouter-device-discovery").start();
    }

    private static void sendDiscoveryRequests(DatagramSocket socket, byte[] request) throws IOException {
        Map<String, InetAddress> destinations = new LinkedHashMap<>();
        destinations.put("255.255.255.255", InetAddress.getByName("255.255.255.255"));
        try {
            for (NetworkInterface networkInterface : Collections.list(NetworkInterface.getNetworkInterfaces())) {
                if (!networkInterface.isUp() || networkInterface.isLoopback()) {
                    continue;
                }
                for (InterfaceAddress interfaceAddress : networkInterface.getInterfaceAddresses()) {
                    InetAddress broadcast = interfaceAddress.getBroadcast();
                    if (broadcast != null) {
                        destinations.put(broadcast.getHostAddress(), broadcast);
                    }
                }
            }
        } catch (SocketException ignored) {
            // Keep the global broadcast fallback.
        }
        for (InetAddress destination : destinations.values()) {
            socket.send(new DatagramPacket(request, request.length, destination, DISCOVERY_PORT));
        }
    }

    private static Device parse(DatagramPacket packet, Set<String> localAddresses) {
        InetAddress address = packet.getAddress();
        if (address == null || address.isLoopbackAddress()
                || localAddresses.contains(address.getHostAddress())) {
            return null;
        }
        String response = new String(packet.getData(), packet.getOffset(), packet.getLength(), StandardCharsets.UTF_8);
        if (!response.startsWith(RESPONSE_PREFIX)) return null;
        String[] parts = response.split("\\|", 3);
        if (parts.length != 3) return null;
        try {
            int port = Integer.parseInt(parts[1]);
            if (port < 1 || port > 65535 || parts[2].trim().isEmpty()) return null;
            return new Device(parts[2].trim(), address, port);
        } catch (NumberFormatException error) {
            return null;
        }
    }

    private static Set<String> localAddresses() {
        Set<String> addresses = new HashSet<>();
        try {
            for (NetworkInterface networkInterface : Collections.list(NetworkInterface.getNetworkInterfaces())) {
                for (InetAddress address : Collections.list(networkInterface.getInetAddresses())) {
                    if (!address.isLoopbackAddress() && !address.isAnyLocalAddress()) {
                        addresses.add(address.getHostAddress());
                    }
                }
            }
        } catch (SocketException ignored) {
            // Discovery can still find remote devices when interface enumeration fails.
        }
        return addresses;
    }

    @Override
    public void close() {
        closed = true;
        DatagramSocket current = socket;
        if (current != null) current.close();
    }
}
