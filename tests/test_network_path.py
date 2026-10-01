from types import SimpleNamespace
from unittest.mock import MagicMock

import pytest

from netaudio import network_path
from netaudio.network_path import (
    IPv4Interface,
    NetworkPathError,
    NetworkPathManager,
    active_ipv4_interfaces,
    preferred_device_address,
)


class RouteSocket:
    def __init__(self, source=None, error=None):
        self.source = source
        self.error = error
        self.destination = None
        self.closed = False

    def connect(self, destination):
        self.destination = destination
        if self.error is not None:
            raise self.error

    def getsockname(self):
        return self.source, 49152

    def close(self):
        self.closed = True


def test_interface_inventory_filters_non_unicast_and_uses_service_priority():
    adapters = [
        SimpleNamespace(
            nice_name="en7",
            ips=[SimpleNamespace(ip="192.0.2.7", network_prefix=24), SimpleNamespace(ip=("fe80::1", 0, 0))],
        ),
        SimpleNamespace(
            nice_name="en0",
            ips=[SimpleNamespace(ip="198.51.100.2", network_prefix=24)],
        ),
        SimpleNamespace(
            nice_name="lo0",
            ips=[SimpleNamespace(ip="127.0.0.1", network_prefix=8)],
        ),
    ]

    assert active_ipv4_interfaces(adapters=adapters, service_order={"en0": 1, "en7": 2}) == (
        IPv4Interface(1, "en0", "198.51.100.2", 24),
        IPv4Interface(2, "en7", "192.0.2.7", 24),
    )


def test_macos_service_order_is_parsed_and_briefly_cached(monkeypatch):
    output = """An asterisk (*) denotes that a network service is disabled.
(1) Wi-Fi
(Hardware Port: Wi-Fi, Device: en0)
(2) Dante
(Hardware Port: USB LAN, Device: en7)
"""
    run = MagicMock(return_value=SimpleNamespace(returncode=0, stdout=output))
    monkeypatch.setattr(network_path.sys, "platform", "darwin")
    monkeypatch.setattr(network_path.subprocess, "run", run)
    monkeypatch.setattr(network_path.time, "monotonic", lambda: 100.0)
    monkeypatch.setattr(network_path, "_service_order_cache", (0.0, {}))

    assert network_path._macos_service_order() == {"en0": 1, "en7": 2}
    assert network_path._macos_service_order() == {"en0": 1, "en7": 2}
    run.assert_called_once_with(
        ["networksetup", "-listnetworkserviceorder"],
        capture_output=True,
        text=True,
        timeout=5,
        check=False,
    )


def test_destination_route_selects_the_matching_source_and_records_generation():
    route_socket = RouteSocket("198.51.100.2")
    interfaces = [
        IPv4Interface(1, "wifi", "192.0.2.2", 24),
        IPv4Interface(2, "dante", "198.51.100.2", 24),
    ]
    manager = NetworkPathManager(interface_provider=lambda: interfaces, socket_factory=lambda *_: route_socket)

    path = manager.resolve("198.51.100.40")

    assert path.interface == interfaces[1]
    assert path.source_address == "198.51.100.2"
    assert path.generation == 1
    assert route_socket.destination == ("198.51.100.40", 9)
    assert route_socket.closed is True


def test_direct_subnet_is_used_when_there_is_no_default_route():
    interface = IPv4Interface(1, "dante", "192.168.50.2", 24)
    manager = NetworkPathManager(
        interface_provider=lambda: [interface],
        socket_factory=lambda *_: RouteSocket(error=OSError("network unreachable")),
    )

    assert manager.resolve("192.168.50.40").interface == interface


def test_interface_generation_changes_on_disconnect_and_reconnect():
    interfaces = [IPv4Interface(1, "dante", "192.168.50.2", 24)]
    manager = NetworkPathManager(
        interface_provider=lambda: interfaces,
        socket_factory=lambda *_: RouteSocket("192.168.50.2"),
    )

    assert manager.resolve("192.168.50.40").generation == 1
    interfaces.clear()
    with pytest.raises(NetworkPathError, match="No eligible"):
        manager.resolve("192.168.50.40")
    interfaces.append(IPv4Interface(1, "dante", "192.168.50.2", 24))
    assert manager.resolve("192.168.50.40").generation == 3


def test_explicit_interface_never_falls_back():
    interfaces = [IPv4Interface(1, "wifi", "192.0.2.2", 24)]
    manager = NetworkPathManager(
        interface_provider=lambda: interfaces,
        socket_factory=lambda *_: RouteSocket("192.0.2.2"),
    )

    with pytest.raises(NetworkPathError, match="selected.*was not found"):
        manager.resolve("192.0.2.40", "selected")


def test_ambiguous_link_local_destination_fails_closed():
    interfaces = [
        IPv4Interface(1, "en7", "169.254.10.2", 16),
        IPv4Interface(2, "en8", "169.254.20.2", 16),
    ]
    manager = NetworkPathManager(
        interface_provider=lambda: interfaces,
        socket_factory=lambda *_: RouteSocket("169.254.10.2"),
    )

    with pytest.raises(NetworkPathError, match="multiple interface-scoped networks"):
        manager.resolve("169.254.30.2")


MULTI_HOMED_ADAPTERS = [
    SimpleNamespace(nice_name="en0", ips=[SimpleNamespace(ip="198.51.100.2", network_prefix=24)]),
    SimpleNamespace(nice_name="en1", ips=[SimpleNamespace(ip="192.0.2.5", network_prefix=24)]),
]


def test_device_address_on_the_selected_interface_subnet_is_preferred():
    addresses = ["198.51.100.9", "192.0.2.7"]

    assert preferred_device_address(addresses, "en1", adapters=MULTI_HOMED_ADAPTERS) == "192.0.2.7"
    assert preferred_device_address(addresses, "en0", adapters=MULTI_HOMED_ADAPTERS) == "198.51.100.9"


def test_device_address_falls_back_to_the_first_ipv4_address():
    addresses = ["fe80::1", "203.0.113.4", "192.0.2.7"]

    assert preferred_device_address(addresses, adapters=MULTI_HOMED_ADAPTERS) == "203.0.113.4"
    assert preferred_device_address(["203.0.113.4"], "en1", adapters=MULTI_HOMED_ADAPTERS) == "203.0.113.4"
    assert preferred_device_address(["fe80::1", "2001:db8::4"], "en1", adapters=MULTI_HOMED_ADAPTERS) is None
    assert preferred_device_address([], adapters=MULTI_HOMED_ADAPTERS) is None
