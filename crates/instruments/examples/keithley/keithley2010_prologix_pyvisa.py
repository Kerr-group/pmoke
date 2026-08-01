#!/usr/bin/env python3
import argparse

import pyvisa


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Query *IDN? from a Keithley 2010 through a Prologix Ethernet controller."
    )
    parser.add_argument("--host", default="10.249.11.17")
    parser.add_argument("--port", type=int, default=1234)
    parser.add_argument("--pad", type=int, default=17)
    parser.add_argument("--timeout-ms", type=int, default=3000)
    parser.add_argument("--backend", default="@py")
    args = parser.parse_args()

    resource_name = f"TCPIP0::{args.host}::{args.port}::SOCKET"
    rm = pyvisa.ResourceManager(args.backend)

    with rm.open_resource(resource_name) as prologix:
        prologix.timeout = args.timeout_ms
        prologix.write_termination = "\n"
        prologix.read_termination = "\n"

        prologix.write("++mode 1")
        prologix.write(f"++addr {args.pad}")
        prologix.write("++auto 0")
        prologix.write("++eoi 1")
        prologix.write("++eos 0")
        prologix.write(f"++read_tmo_ms {args.timeout_ms}")

        prologix.write("*IDN?")
        prologix.write("++read eoi")
        print(prologix.read().strip())


if __name__ == "__main__":
    main()
