# SoftNpu

SoftNpu is a software network processing unit. The main thing SoftNpu
implementations do is provide an I/O harness and management interface for 
[x4c](https://github.com/oxidecomputer/p4)
compiled P4 programs. I/O harnesses provide a way to get packets in and out of
P4 pipeline programs and management interfaces provide a means to manage P4
table state from a control plane program.

This repository contains the following.

- Management message definitions for SoftNpu implementations.
- A standalone SoftNpu implementation.

Two SoftNpu implementations currently exist. 

1. The standalone implementation in this repository
2. A propolis ASIC emulator.
