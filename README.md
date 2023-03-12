![GitHub top language](https://img.shields.io/github/languages/top/rust-util-collections/rt-evm)
[![Minimum rustc version](https://img.shields.io/badge/rustc-1.65+-lightgray.svg)](https://github.com/rust-random/rand#rust-version-requirements)
![GitHub Workflow Status](https://img.shields.io/github/actions/workflow/status/rust-util-collections/rt-evm/rust.yml?branch=master)

# rt-evm

**U**til **C**ollections of **EVM**.

A simple development framework for creating EVM-compatible chains.

```mermaid
graph TD
    LIB --> |HTTP or gRPC| COLLECTOR
    LIB --> |UDP| AGENT
    AGENT --> |gRPC| COLLECTOR
    SDK --> |UDP| AGENT
    SDK --> |HTTP or gRPC| COLLECTOR
    COLLECTOR --> STORE
    COLLECTOR --> |gRPC| PLUGIN
    PLUGIN --> STORE
    QUERY --> STORE
    QUERY --> |gRPC| PLUGIN
    UI --> |HTTP| QUERY
    subgraph Application Host
        subgraph User Application
            LIB
            SDK
        end
        AGENT
    end
```

### Gratitude

Thanks to all the people who already contributed!

<a href="https://github.com/rust-util-collections/rt-evm/graphs/contributors">
  <img src="https://contributors-img.web.app/image?repo=rust-util-collections/rt-evm"/>
</a>
