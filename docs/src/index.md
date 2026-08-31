# Axioval Engine

Axioval Engine validates federated engineering data without making any source format or geometry kernel its internal model.

The runtime compiles normalized Axioval packages into trusted capability invocations. Source adapters provide semantic objects and evidence. Geometry adapters provide optional exact geometric evidence. Findings retain source identity and provenance.

## Why a dedicated IR?

IFC remains an important adapter, but its schema inheritance, STEP identity, file-centric lifetime and serialization history are not suitable as a universal runtime contract. A dedicated IR also admits proprietary CAD, database-backed digital twins, ICDD collections, and future IFCX-style layered compositions.

## Current status

The engine is being extracted from the production Solibri compatibility implementation. The migration ledger records what is actually cut over; unchecked entries are not claims of support.
