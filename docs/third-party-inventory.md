# Componentes de terceiros

O Codisseu usa dependências Rust, fontes, ícones e outros componentes com suas
próprias licenças. O Cargo.lock fixa as versões dos pacotes. Os avisos e textos
de licença dos recursos incorporados estão em reference/public e vendor.

O .deb core inclui seu inventário e os avisos correspondentes em
/opt/codisseu/share/codisseu/notices. Motores externos preparados separadamente conservam suas
próprias obrigações; eles não acompanham esta primeira distribuição pública.

Este resumo não republica relatórios de instalações pessoais. O bloco abaixo é
o inventário declarativo dos metadados Cargo, quando disponível; não é uma
declaração de que todo pacote listado integra o executável Linux.

<!-- BEGIN CARGO METADATA INVENTORY -->

## Inventário gerado dos metadados Cargo

Cargo.lock SHA-256: `82115345551efcf7a8523c68de93d11b459cebfc8c0dc07053ad2098045026e1`.
Pacotes terceiros: **717**. Campo `license` ausente: **0**.

Origem: `cargo metadata --locked --format-version 1`, workspace completo, sem filtro de plataforma. Inclui dependências de testes, build, outros alvos e pacotes resolvidos que podem não estar no executável Linux. Não é uma lista de componentes comprovadamente distribuídos nem auditoria manual de todos os arquivos. Checksums dos pacotes estão no Cargo.lock.

Expressões com fontes, Unicode, copyleft ou licença customizada requerem atenção específica:

- `colored 2.2.0`: `MPL-2.0`.
- `epaint_default_fonts 0.37.0`: `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`.
- `icu_collections 2.4.0`: `Unicode-3.0`.
- `icu_locale_core 2.4.0`: `Unicode-3.0`.
- `icu_normalizer 2.4.0`: `Unicode-3.0`.
- `icu_normalizer_data 2.4.0`: `Unicode-3.0`.
- `icu_properties 2.4.0`: `Unicode-3.0`.
- `icu_properties_data 2.4.0`: `Unicode-3.0`.
- `icu_provider 2.4.0`: `Unicode-3.0`.
- `litemap 0.8.3`: `Unicode-3.0`.
- `potential_utf 0.1.6`: `Unicode-3.0`.
- `r-efi 5.3.0`: `MIT OR Apache-2.0 OR LGPL-2.1-or-later`.
- `r-efi 6.0.0`: `MIT OR Apache-2.0 OR LGPL-2.1-or-later`.
- `self_cell 1.3.0`: `Apache-2.0 OR GPL-2.0-only`.
- `tinystr 0.8.5`: `Unicode-3.0`.
- `unicode-ident 1.0.27`: `(MIT OR Apache-2.0) AND Unicode-3.0`.
- `unicode-normalization 0.1.26`: `(MIT OR Apache-2.0) AND Unicode-3.0`.
- `writeable 0.6.4`: `Unicode-3.0`.
- `yoke 0.8.4`: `Unicode-3.0`.
- `yoke-derive 0.8.4`: `Unicode-3.0`.
- `zerofrom 0.1.8`: `Unicode-3.0`.
- `zerofrom-derive 0.1.8`: `Unicode-3.0`.
- `zerotrie 0.2.5`: `Unicode-3.0`.
- `zerovec 0.11.9`: `Unicode-3.0`.
- `zerovec-derive 0.11.7`: `Unicode-3.0`.

A presença de `OR GPL`/`OR LGPL` em uma expressão com alternativa permissiva não implica escolher GPL/LGPL. Expressões legadas com `/` são reproduzidas sem reinterpretá-las. A seleção e os avisos necessários devem ser registrados ao empacotar.

| Pacote / origem da versão | Licença declarada | Rust mínimo declarado |
|---|---|---|
| [accesskit 0.25.1](https://crates.io/crates/accesskit/0.25.1) | `MIT OR Apache-2.0` | 1.87 |
| [accesskit_atspi_common 0.21.0](https://crates.io/crates/accesskit_atspi_common/0.21.0) | `MIT OR Apache-2.0` | 1.87 |
| [accesskit_consumer 0.39.1](https://crates.io/crates/accesskit_consumer/0.39.1) | `MIT OR Apache-2.0` | 1.87 |
| [accesskit_ios 0.2.1](https://crates.io/crates/accesskit_ios/0.2.1) | `MIT OR Apache-2.0` | 1.87 |
| [accesskit_macos 0.27.1](https://crates.io/crates/accesskit_macos/0.27.1) | `MIT OR Apache-2.0` | 1.87 |
| [accesskit_unix 0.24.0](https://crates.io/crates/accesskit_unix/0.24.0) | `MIT OR Apache-2.0` | 1.87 |
| [accesskit_windows 0.35.1](https://crates.io/crates/accesskit_windows/0.35.1) | `MIT OR Apache-2.0` | 1.87 |
| [accesskit_winit 0.34.1](https://crates.io/crates/accesskit_winit/0.34.1) | `Apache-2.0` | 1.87 |
| [adler2 2.0.1](https://crates.io/crates/adler2/2.0.1) | `0BSD OR MIT OR Apache-2.0` | não declarado |
| [aes 0.8.4](https://crates.io/crates/aes/0.8.4) | `MIT OR Apache-2.0` | 1.56 |
| [ahash 0.8.12](https://crates.io/crates/ahash/0.8.12) | `MIT OR Apache-2.0` | 1.60.0 |
| [aho-corasick 1.1.5](https://crates.io/crates/aho-corasick/1.1.5) | `Unlicense OR MIT` | 1.60.0 |
| [alloc-no-stdlib 3.0.0](https://crates.io/crates/alloc-no-stdlib/3.0.0) | `BSD-3-Clause` | não declarado |
| [alloc-stdlib 0.3.0](https://crates.io/crates/alloc-stdlib/0.3.0) | `BSD-3-Clause` | não declarado |
| [allocator-api2 0.2.21](https://crates.io/crates/allocator-api2/0.2.21) | `MIT OR Apache-2.0` | 1.63 |
| [android-activity 0.6.1](https://crates.io/crates/android-activity/0.6.1) | `MIT OR Apache-2.0` | 1.85.0 |
| [android-properties 0.2.2](https://crates.io/crates/android-properties/0.2.2) | `MIT` | não declarado |
| [android_system_properties 0.1.6](https://crates.io/crates/android_system_properties/0.1.6) | `MIT OR Apache-2.0` | não declarado |
| [anyhow 1.0.104](https://crates.io/crates/anyhow/1.0.104) | `MIT OR Apache-2.0` | 1.68 |
| [ar_archive_writer 0.5.3](https://crates.io/crates/ar_archive_writer/0.5.3) | `Apache-2.0 WITH LLVM-exception` | 1.88.0 |
| [arboard 3.7.0](https://crates.io/crates/arboard/3.7.0) | `MIT OR Apache-2.0` | 1.71.0 |
| [arrayvec 0.7.8](https://crates.io/crates/arrayvec/0.7.8) | `MIT OR Apache-2.0` | 1.51 |
| [as-raw-xcb-connection 1.0.1](https://crates.io/crates/as-raw-xcb-connection/1.0.1) | `MIT OR Apache-2.0` | não declarado |
| [ash 0.38.0+1.3.281](https://crates.io/crates/ash/0.38.0+1.3.281) | `MIT OR Apache-2.0` | 1.69.0 |
| [async-broadcast 0.7.2](https://crates.io/crates/async-broadcast/0.7.2) | `MIT OR Apache-2.0` | 1.60 |
| [async-channel 2.5.0](https://crates.io/crates/async-channel/2.5.0) | `Apache-2.0 OR MIT` | 1.60 |
| [async-compression 0.4.51](https://crates.io/crates/async-compression/0.4.51) | `MIT OR Apache-2.0` | 1.83 |
| [async-executor 1.14.0](https://crates.io/crates/async-executor/1.14.0) | `Apache-2.0 OR MIT` | 1.65 |
| [async-fs 2.2.0](https://crates.io/crates/async-fs/2.2.0) | `Apache-2.0 OR MIT` | 1.71 |
| [async-io 2.6.0](https://crates.io/crates/async-io/2.6.0) | `Apache-2.0 OR MIT` | 1.71 |
| [async-lock 3.4.2](https://crates.io/crates/async-lock/3.4.2) | `Apache-2.0 OR MIT` | 1.85 |
| [async-process 2.5.0](https://crates.io/crates/async-process/2.5.0) | `Apache-2.0 OR MIT` | 1.71 |
| [async-recursion 1.2.0](https://crates.io/crates/async-recursion/1.2.0) | `MIT OR Apache-2.0` | 1.71 |
| [async-signal 0.2.14](https://crates.io/crates/async-signal/0.2.14) | `Apache-2.0 OR MIT` | 1.85 |
| [async-task 4.7.1](https://crates.io/crates/async-task/4.7.1) | `Apache-2.0 OR MIT` | 1.57 |
| [async-trait 0.1.92](https://crates.io/crates/async-trait/0.1.92) | `MIT OR Apache-2.0` | 1.71 |
| [atoi 2.0.0](https://crates.io/crates/atoi/2.0.0) | `MIT` | 1.57.0 |
| [atomic-waker 1.1.2](https://crates.io/crates/atomic-waker/1.1.2) | `Apache-2.0 OR MIT` | 1.36 |
| [atspi 0.29.0](https://crates.io/crates/atspi/0.29.0) | `Apache-2.0 OR MIT` | 1.77.2 |
| [atspi-common 0.13.0](https://crates.io/crates/atspi-common/0.13.0) | `Apache-2.0 OR MIT` | 1.77.2 |
| [atspi-proxies 0.13.0](https://crates.io/crates/atspi-proxies/0.13.0) | `Apache-2.0 OR MIT` | 1.77.2 |
| [autocfg 1.5.1](https://crates.io/crates/autocfg/1.5.1) | `Apache-2.0 OR MIT` | 1.0 |
| [base64 0.22.1](https://crates.io/crates/base64/0.22.1) | `MIT OR Apache-2.0` | 1.48.0 |
| [base64 0.23.1](https://crates.io/crates/base64/0.23.1) | `MIT OR Apache-2.0` | 1.71.0 |
| [base64ct 1.8.3](https://crates.io/crates/base64ct/1.8.3) | `Apache-2.0 OR MIT` | 1.85 |
| [bigdecimal 0.4.11](https://crates.io/crates/bigdecimal/0.4.11) | `MIT/Apache-2.0` | não declarado |
| [bit-set 0.10.0](https://crates.io/crates/bit-set/0.10.0) | `Apache-2.0 OR MIT` | 1.82 |
| [bit-vec 0.9.1](https://crates.io/crates/bit-vec/0.9.1) | `Apache-2.0 OR MIT` | 1.82 |
| [bitflags 1.3.2](https://crates.io/crates/bitflags/1.3.2) | `MIT/Apache-2.0` | não declarado |
| [bitflags 2.13.2](https://crates.io/crates/bitflags/2.13.2) | `MIT OR Apache-2.0` | 1.56.0 |
| [bitvec 1.1.1](https://crates.io/crates/bitvec/1.1.1) | `MIT` | 1.56 |
| [block-buffer 0.10.4](https://crates.io/crates/block-buffer/0.10.4) | `MIT OR Apache-2.0` | não declarado |
| [block-buffer 0.12.1](https://crates.io/crates/block-buffer/0.12.1) | `MIT OR Apache-2.0` | 1.85 |
| [block-padding 0.3.3](https://crates.io/crates/block-padding/0.3.3) | `MIT OR Apache-2.0` | 1.56 |
| [block2 0.5.1](https://crates.io/crates/block2/0.5.1) | `MIT` | 1.60 |
| [block2 0.6.2](https://crates.io/crates/block2/0.6.2) | `MIT` | 1.71 |
| [blocking 1.7.0](https://crates.io/crates/blocking/1.7.0) | `Apache-2.0 OR MIT` | 1.68 |
| [brotli 9.1.0](https://crates.io/crates/brotli/9.1.0) | `BSD-3-Clause AND MIT` | 1.59.0 |
| [brotli-decompressor 6.2.0](https://crates.io/crates/brotli-decompressor/6.2.0) | `BSD-3-Clause/MIT` | não declarado |
| [bson 2.15.0](https://crates.io/crates/bson/2.15.0) | `MIT` | não declarado |
| [bumpalo 3.20.3](https://crates.io/crates/bumpalo/3.20.3) | `MIT OR Apache-2.0` | 1.71.1 |
| [bytemuck 1.25.2](https://crates.io/crates/bytemuck/1.25.2) | `Zlib OR Apache-2.0 OR MIT` | não declarado |
| [bytemuck_derive 1.12.1](https://crates.io/crates/bytemuck_derive/1.12.1) | `Zlib OR Apache-2.0 OR MIT` | 1.71 |
| [byteorder 1.5.0](https://crates.io/crates/byteorder/1.5.0) | `Unlicense OR MIT` | 1.60 |
| [byteorder-lite 0.1.0](https://crates.io/crates/byteorder-lite/0.1.0) | `Unlicense OR MIT` | 1.60 |
| [bytes 1.12.1](https://crates.io/crates/bytes/1.12.1) | `MIT` | 1.57 |
| [calloop 0.13.0](https://crates.io/crates/calloop/0.13.0) | `MIT` | 1.63.0 |
| [calloop 0.14.5](https://crates.io/crates/calloop/0.14.5) | `MIT` | 1.86.0 |
| [calloop-wayland-source 0.3.0](https://crates.io/crates/calloop-wayland-source/0.3.0) | `MIT` | 1.65.0 |
| [calloop-wayland-source 0.4.1](https://crates.io/crates/calloop-wayland-source/0.4.1) | `MIT` | 1.65.0 |
| [cbc 0.1.2](https://crates.io/crates/cbc/0.1.2) | `MIT OR Apache-2.0` | 1.56 |
| [cc 1.9.0](https://crates.io/crates/cc/1.9.0) | `MIT OR Apache-2.0` | 1.65.0 |
| [cfg-if 1.0.5](https://crates.io/crates/cfg-if/1.0.5) | `MIT OR Apache-2.0` | 1.32 |
| [cfg_aliases 0.1.1](https://crates.io/crates/cfg_aliases/0.1.1) | `MIT` | não declarado |
| [cfg_aliases 0.2.2](https://crates.io/crates/cfg_aliases/0.2.2) | `MIT` | não declarado |
| [cgl 0.3.2](https://crates.io/crates/cgl/0.3.2) | `MIT / Apache-2.0` | não declarado |
| [chacha20 0.10.2](https://crates.io/crates/chacha20/0.10.2) | `MIT OR Apache-2.0` | 1.85 |
| [chrono 0.4.45](https://crates.io/crates/chrono/0.4.45) | `MIT OR Apache-2.0` | 1.62.0 |
| [cipher 0.4.4](https://crates.io/crates/cipher/0.4.4) | `MIT OR Apache-2.0` | 1.56 |
| [clipboard-win 5.4.1](https://crates.io/crates/clipboard-win/5.4.1) | `BSL-1.0` | não declarado |
| [codespan-reporting 0.13.1](https://crates.io/crates/codespan-reporting/0.13.1) | `Apache-2.0` | 1.67 |
| [color 0.3.3](https://crates.io/crates/color/0.3.3) | `Apache-2.0 OR MIT` | 1.82 |
| [color_quant 1.1.0](https://crates.io/crates/color_quant/1.1.0) | `MIT` | não declarado |
| [colored 2.2.0](https://crates.io/crates/colored/2.2.0) | `MPL-2.0` | 1.70 |
| [combine 4.6.9](https://crates.io/crates/combine/4.6.9) | `MIT` | não declarado |
| [compression-codecs 0.4.46](https://crates.io/crates/compression-codecs/0.4.46) | `MIT OR Apache-2.0` | 1.83 |
| [compression-core 0.4.34](https://crates.io/crates/compression-core/0.4.34) | `MIT OR Apache-2.0` | não declarado |
| [concurrent-queue 2.5.0](https://crates.io/crates/concurrent-queue/2.5.0) | `Apache-2.0 OR MIT` | 1.60 |
| [const-oid 0.10.2](https://crates.io/crates/const-oid/0.10.2) | `Apache-2.0 OR MIT` | 1.85 |
| [const-oid 0.9.6](https://crates.io/crates/const-oid/0.9.6) | `Apache-2.0 OR MIT` | 1.57 |
| [const-random 0.1.18](https://crates.io/crates/const-random/0.1.18) | `MIT OR Apache-2.0` | não declarado |
| [const-random-macro 0.1.16](https://crates.io/crates/const-random-macro/0.1.16) | `MIT OR Apache-2.0` | não declarado |
| [convert_case 0.10.0](https://crates.io/crates/convert_case/0.10.0) | `MIT` | não declarado |
| [cookie 0.18.2](https://crates.io/crates/cookie/0.18.2) | `MIT OR Apache-2.0` | 1.56 |
| [cookie_store 0.22.1](https://crates.io/crates/cookie_store/0.22.1) | `MIT OR Apache-2.0` | 1.88.0 |
| [core-foundation 0.10.1](https://crates.io/crates/core-foundation/0.10.1) | `MIT OR Apache-2.0` | 1.65 |
| [core-foundation 0.9.4](https://crates.io/crates/core-foundation/0.9.4) | `MIT OR Apache-2.0` | não declarado |
| [core-foundation-sys 0.8.7](https://crates.io/crates/core-foundation-sys/0.8.7) | `MIT OR Apache-2.0` | não declarado |
| [core-graphics 0.23.2](https://crates.io/crates/core-graphics/0.23.2) | `MIT OR Apache-2.0` | não declarado |
| [core-graphics-types 0.1.3](https://crates.io/crates/core-graphics-types/0.1.3) | `MIT OR Apache-2.0` | não declarado |
| [cpufeatures 0.2.17](https://crates.io/crates/cpufeatures/0.2.17) | `MIT OR Apache-2.0` | não declarado |
| [cpufeatures 0.3.1](https://crates.io/crates/cpufeatures/0.3.1) | `MIT OR Apache-2.0` | 1.85 |
| [crc 3.4.0](https://crates.io/crates/crc/3.4.0) | `MIT OR Apache-2.0` | 1.83 |
| [crc-catalog 2.5.0](https://crates.io/crates/crc-catalog/2.5.0) | `MIT OR Apache-2.0` | não declarado |
| [crc32fast 1.5.3](https://crates.io/crates/crc32fast/1.5.3) | `MIT OR Apache-2.0` | 1.63 |
| [critical-section 1.2.0](https://crates.io/crates/critical-section/1.2.0) | `MIT OR Apache-2.0` | não declarado |
| [crossbeam-channel 0.5.17](https://crates.io/crates/crossbeam-channel/0.5.17) | `MIT OR Apache-2.0` | 1.60 |
| [crossbeam-deque 0.8.8](https://crates.io/crates/crossbeam-deque/0.8.8) | `MIT OR Apache-2.0` | 1.61 |
| [crossbeam-epoch 0.9.21](https://crates.io/crates/crossbeam-epoch/0.9.21) | `MIT OR Apache-2.0` | 1.61 |
| [crossbeam-queue 0.3.14](https://crates.io/crates/crossbeam-queue/0.3.14) | `MIT OR Apache-2.0` | 1.60 |
| [crossbeam-utils 0.8.23](https://crates.io/crates/crossbeam-utils/0.8.23) | `MIT OR Apache-2.0` | 1.60 |
| [crunchy 0.2.4](https://crates.io/crates/crunchy/0.2.4) | `MIT` | não declarado |
| [crypto-common 0.1.7](https://crates.io/crates/crypto-common/0.1.7) | `MIT OR Apache-2.0` | não declarado |
| [crypto-common 0.2.2](https://crates.io/crates/crypto-common/0.2.2) | `MIT OR Apache-2.0` | 1.85 |
| [cursor-icon 1.2.0](https://crates.io/crates/cursor-icon/1.2.0) | `MIT OR Apache-2.0 OR Zlib` | 1.65.0 |
| [darling 0.24.1](https://crates.io/crates/darling/0.24.1) | `MIT` | 1.88.0 |
| [darling_core 0.24.1](https://crates.io/crates/darling_core/0.24.1) | `MIT` | 1.88.0 |
| [darling_macro 0.24.1](https://crates.io/crates/darling_macro/0.24.1) | `MIT` | 1.88.0 |
| [data-encoding 2.11.1](https://crates.io/crates/data-encoding/2.11.1) | `MIT` | 1.48 |
| [dbus 0.9.12](https://crates.io/crates/dbus/0.9.12) | `Apache-2.0/MIT` | não declarado |
| [dbus-secret-service 4.1.0](https://crates.io/crates/dbus-secret-service/4.1.0) | `MIT OR Apache-2.0` | 1.78 |
| [der 0.7.10](https://crates.io/crates/der/0.7.10) | `Apache-2.0 OR MIT` | 1.65 |
| [deranged 0.5.9](https://crates.io/crates/deranged/0.5.9) | `MIT OR Apache-2.0` | 1.85.0 |
| [derive-syn-parse 0.2.0](https://crates.io/crates/derive-syn-parse/0.2.0) | `MIT OR Apache-2.0` | não declarado |
| [derive-where 1.7.0](https://crates.io/crates/derive-where/1.7.0) | `MIT OR Apache-2.0` | 1.57 |
| [derive_more 2.1.1](https://crates.io/crates/derive_more/2.1.1) | `MIT` | 1.81.0 |
| [derive_more-impl 2.1.1](https://crates.io/crates/derive_more-impl/2.1.1) | `MIT` | 1.81.0 |
| [dify 0.8.0](https://crates.io/crates/dify/0.8.0) | `MIT` | não declarado |
| [digest 0.10.7](https://crates.io/crates/digest/0.10.7) | `MIT OR Apache-2.0` | não declarado |
| [digest 0.11.3](https://crates.io/crates/digest/0.11.3) | `MIT OR Apache-2.0` | 1.85 |
| [dispatch 0.2.0](https://crates.io/crates/dispatch/0.2.0) | `MIT` | não declarado |
| [dispatch2 0.3.1](https://crates.io/crates/dispatch2/0.3.1) | `Zlib OR Apache-2.0 OR MIT` | 1.71 |
| [displaydoc 0.2.7](https://crates.io/crates/displaydoc/0.2.7) | `MIT OR Apache-2.0` | 1.71.0 |
| [dlib 0.5.3](https://crates.io/crates/dlib/0.5.3) | `MIT` | 1.56 |
| [document-features 0.2.12](https://crates.io/crates/document-features/0.2.12) | `MIT OR Apache-2.0` | 1.56 |
| [dotenvy 0.15.7](https://crates.io/crates/dotenvy/0.15.7) | `MIT` | 1.56.1 |
| [downcast-rs 1.2.1](https://crates.io/crates/downcast-rs/1.2.1) | `MIT/Apache-2.0` | não declarado |
| [dpi 0.1.2](https://crates.io/crates/dpi/0.1.2) | `Apache-2.0 AND MIT` | 1.73 |
| [ecolor 0.37.0](https://crates.io/crates/ecolor/0.37.0) | `MIT OR Apache-2.0` | 1.95 |
| [eframe 0.37.0](https://crates.io/crates/eframe/0.37.0) | `MIT OR Apache-2.0` | 1.95 |
| [egui 0.37.0](https://crates.io/crates/egui/0.37.0) | `MIT OR Apache-2.0` | 1.95 |
| [egui-wgpu 0.37.0](https://crates.io/crates/egui-wgpu/0.37.0) | `MIT OR Apache-2.0` | 1.95 |
| [egui_glow 0.37.0](https://crates.io/crates/egui_glow/0.37.0) | `MIT OR Apache-2.0` | 1.95 |
| [egui_kittest 0.37.0](https://crates.io/crates/egui_kittest/0.37.0) | `MIT OR Apache-2.0` | 1.95 |
| [either 1.19.0](https://crates.io/crates/either/1.19.0) | `MIT OR Apache-2.0` | 1.63.0 |
| [emath 0.37.0](https://crates.io/crates/emath/0.37.0) | `MIT OR Apache-2.0` | 1.95 |
| [endi 1.1.1](https://crates.io/crates/endi/1.1.1) | `MIT` | 1.60 |
| [enumflags2 0.7.12](https://crates.io/crates/enumflags2/0.7.12) | `MIT OR Apache-2.0` | 1.56 |
| [enumflags2_derive 0.7.12](https://crates.io/crates/enumflags2_derive/0.7.12) | `MIT OR Apache-2.0` | 1.56 |
| [epaint 0.37.0](https://crates.io/crates/epaint/0.37.0) | `MIT OR Apache-2.0` | 1.95 |
| [epaint_default_fonts 0.37.0](https://crates.io/crates/epaint_default_fonts/0.37.0) | `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0` | 1.95 |
| [equivalent 1.0.3](https://crates.io/crates/equivalent/1.0.3) | `Apache-2.0 OR MIT` | 1.6 |
| [errno 0.3.14](https://crates.io/crates/errno/0.3.14) | `MIT OR Apache-2.0` | 1.56 |
| [error-code 3.4.0](https://crates.io/crates/error-code/3.4.0) | `BSL-1.0` | não declarado |
| [etcetera 0.8.0](https://crates.io/crates/etcetera/0.8.0) | `MIT OR Apache-2.0` | não declarado |
| [euclid 0.22.14](https://crates.io/crates/euclid/0.22.14) | `MIT OR Apache-2.0` | 1.63.0 |
| [event-listener 5.4.2](https://crates.io/crates/event-listener/5.4.2) | `Apache-2.0 OR MIT` | 1.60 |
| [event-listener-strategy 0.5.4](https://crates.io/crates/event-listener-strategy/0.5.4) | `Apache-2.0 OR MIT` | 1.60 |
| [fallible-iterator 0.3.0](https://crates.io/crates/fallible-iterator/0.3.0) | `MIT/Apache-2.0` | não declarado |
| [fallible-streaming-iterator 0.1.9](https://crates.io/crates/fallible-streaming-iterator/0.1.9) | `MIT/Apache-2.0` | não declarado |
| [fastrand 2.5.0](https://crates.io/crates/fastrand/2.5.0) | `Apache-2.0 OR MIT` | 1.63 |
| [fax 0.2.7](https://crates.io/crates/fax/0.2.7) | `MIT` | 1.71 |
| [fdeflate 0.3.7](https://crates.io/crates/fdeflate/0.3.7) | `MIT OR Apache-2.0` | 1.67.0 |
| [fearless_simd 0.4.1](https://crates.io/crates/fearless_simd/0.4.1) | `Apache-2.0 OR MIT` | 1.88 |
| [filedescriptor 0.8.3](https://crates.io/crates/filedescriptor/0.8.3) | `MIT` | não declarado |
| [find-msvc-tools 0.1.15](https://crates.io/crates/find-msvc-tools/0.1.15) | `MIT OR Apache-2.0` | 1.65.0 |
| [fixedbitset 0.5.7](https://crates.io/crates/fixedbitset/0.5.7) | `MIT OR Apache-2.0` | 1.56 |
| [flate2 1.1.10](https://crates.io/crates/flate2/1.1.10) | `MIT OR Apache-2.0` | 1.67.0 |
| [foldhash 0.1.5](https://crates.io/crates/foldhash/0.1.5) | `Zlib` | 1.60 |
| [foldhash 0.2.0](https://crates.io/crates/foldhash/0.2.0) | `Zlib` | 1.60 |
| [font-types 0.12.7](https://crates.io/crates/font-types/0.12.7) | `MIT OR Apache-2.0` | 1.85 |
| [foreign-types 0.5.0](https://crates.io/crates/foreign-types/0.5.0) | `MIT/Apache-2.0` | não declarado |
| [foreign-types-macros 0.2.4](https://crates.io/crates/foreign-types-macros/0.2.4) | `MIT/Apache-2.0` | não declarado |
| [foreign-types-shared 0.3.1](https://crates.io/crates/foreign-types-shared/0.3.1) | `MIT/Apache-2.0` | não declarado |
| [form_urlencoded 1.2.2](https://crates.io/crates/form_urlencoded/1.2.2) | `MIT OR Apache-2.0` | 1.51 |
| [funty 2.0.0](https://crates.io/crates/funty/2.0.0) | `MIT` | não declarado |
| [futures-channel 0.3.34](https://crates.io/crates/futures-channel/0.3.34) | `MIT OR Apache-2.0` | 1.71 |
| [futures-core 0.3.34](https://crates.io/crates/futures-core/0.3.34) | `MIT OR Apache-2.0` | 1.36 |
| [futures-intrusive 0.5.0](https://crates.io/crates/futures-intrusive/0.5.0) | `MIT OR Apache-2.0` | não declarado |
| [futures-io 0.3.34](https://crates.io/crates/futures-io/0.3.34) | `MIT OR Apache-2.0` | 1.36 |
| [futures-lite 2.6.1](https://crates.io/crates/futures-lite/2.6.1) | `Apache-2.0 OR MIT` | 1.60 |
| [futures-macro 0.3.34](https://crates.io/crates/futures-macro/0.3.34) | `MIT OR Apache-2.0` | 1.71 |
| [futures-sink 0.3.34](https://crates.io/crates/futures-sink/0.3.34) | `MIT OR Apache-2.0` | 1.36 |
| [futures-task 0.3.34](https://crates.io/crates/futures-task/0.3.34) | `MIT OR Apache-2.0` | 1.71 |
| [futures-util 0.3.34](https://crates.io/crates/futures-util/0.3.34) | `MIT OR Apache-2.0` | 1.71 |
| [generic-array 0.14.7](https://crates.io/crates/generic-array/0.14.7) | `MIT` | não declarado |
| [gethostname 1.1.0](https://crates.io/crates/gethostname/1.1.0) | `Apache-2.0` | 1.71 |
| [getopts 0.2.24](https://crates.io/crates/getopts/0.2.24) | `MIT OR Apache-2.0` | 1.66 |
| [getrandom 0.2.17](https://crates.io/crates/getrandom/0.2.17) | `MIT OR Apache-2.0` | não declarado |
| [getrandom 0.3.4](https://crates.io/crates/getrandom/0.3.4) | `MIT OR Apache-2.0` | 1.63 |
| [getrandom 0.4.3](https://crates.io/crates/getrandom/0.4.3) | `MIT OR Apache-2.0` | 1.85 |
| [gif 0.14.2](https://crates.io/crates/gif/0.14.2) | `MIT OR Apache-2.0` | 1.62 |
| [gl_generator 0.14.0](https://crates.io/crates/gl_generator/0.14.0) | `Apache-2.0` | não declarado |
| [glifo 0.3.0](https://crates.io/crates/glifo/0.3.0) | `Apache-2.0 OR MIT` | 1.88 |
| [glow 0.17.0](https://crates.io/crates/glow/0.17.0) | `MIT OR Apache-2.0 OR Zlib` | não declarado |
| [glutin 0.32.3](https://crates.io/crates/glutin/0.32.3) | `Apache-2.0` | 1.71.0 |
| [glutin-winit 0.5.0](https://crates.io/crates/glutin-winit/0.5.0) | `MIT` | 1.70.0 |
| [glutin_egl_sys 0.7.1](https://crates.io/crates/glutin_egl_sys/0.7.1) | `Apache-2.0` | 1.70.0 |
| [glutin_glx_sys 0.6.1](https://crates.io/crates/glutin_glx_sys/0.6.1) | `Apache-2.0` | 1.70.0 |
| [glutin_wgl_sys 0.6.1](https://crates.io/crates/glutin_wgl_sys/0.6.1) | `Apache-2.0` | 1.70.0 |
| [gpu-allocator 0.28.0](https://crates.io/crates/gpu-allocator/0.28.0) | `MIT OR Apache-2.0` | 1.71 |
| [guillotiere 0.7.0](https://crates.io/crates/guillotiere/0.7.0) | `MIT/Apache-2.0` | não declarado |
| [half 2.7.1](https://crates.io/crates/half/2.7.1) | `MIT OR Apache-2.0` | 1.81 |
| [harfrust 0.12.0](https://crates.io/crates/harfrust/0.12.0) | `MIT` | 1.85 |
| [hashbrown 0.15.5](https://crates.io/crates/hashbrown/0.15.5) | `MIT OR Apache-2.0` | 1.65.0 |
| [hashbrown 0.16.1](https://crates.io/crates/hashbrown/0.16.1) | `MIT OR Apache-2.0` | 1.65.0 |
| [hashbrown 0.17.1](https://crates.io/crates/hashbrown/0.17.1) | `MIT OR Apache-2.0` | 1.85.0 |
| [hashlink 0.10.0](https://crates.io/crates/hashlink/0.10.0) | `MIT OR Apache-2.0` | 1.65 |
| [hermit-abi 0.5.3](https://crates.io/crates/hermit-abi/0.5.3) | `MIT OR Apache-2.0` | não declarado |
| [hex 0.4.3](https://crates.io/crates/hex/0.4.3) | `MIT OR Apache-2.0` | não declarado |
| [hickory-net 0.26.3](https://crates.io/crates/hickory-net/0.26.3) | `MIT OR Apache-2.0` | 1.88 |
| [hickory-proto 0.26.3](https://crates.io/crates/hickory-proto/0.26.3) | `MIT OR Apache-2.0` | 1.88 |
| [hickory-resolver 0.26.3](https://crates.io/crates/hickory-resolver/0.26.3) | `MIT OR Apache-2.0` | 1.88 |
| [hkdf 0.12.4](https://crates.io/crates/hkdf/0.12.4) | `MIT OR Apache-2.0` | não declarado |
| [hmac 0.12.1](https://crates.io/crates/hmac/0.12.1) | `MIT OR Apache-2.0` | não declarado |
| [home 0.5.12](https://crates.io/crates/home/0.5.12) | `MIT OR Apache-2.0` | 1.88 |
| [http 1.5.0](https://crates.io/crates/http/1.5.0) | `MIT OR Apache-2.0` | 1.57.0 |
| [http-body 1.1.0](https://crates.io/crates/http-body/1.1.0) | `MIT` | 1.61 |
| [http-body-util 0.1.5](https://crates.io/crates/http-body-util/0.1.5) | `MIT` | 1.61 |
| [httparse 1.10.1](https://crates.io/crates/httparse/1.10.1) | `MIT OR Apache-2.0` | não declarado |
| [hybrid-array 0.4.15](https://crates.io/crates/hybrid-array/0.4.15) | `MIT OR Apache-2.0` | 1.85 |
| [hyper 1.12.0](https://crates.io/crates/hyper/1.12.0) | `MIT` | 1.63 |
| [hyper-rustls 0.27.10](https://crates.io/crates/hyper-rustls/0.27.10) | `Apache-2.0 OR ISC OR MIT` | 1.85 |
| [hyper-util 0.1.21](https://crates.io/crates/hyper-util/0.1.21) | `MIT` | 1.85 |
| [iana-time-zone 0.1.65](https://crates.io/crates/iana-time-zone/0.1.65) | `MIT OR Apache-2.0` | 1.62.0 |
| [iana-time-zone-haiku 0.1.2](https://crates.io/crates/iana-time-zone-haiku/0.1.2) | `MIT OR Apache-2.0` | não declarado |
| [icu_collections 2.4.0](https://crates.io/crates/icu_collections/2.4.0) | `Unicode-3.0` | 1.88 |
| [icu_locale_core 2.4.0](https://crates.io/crates/icu_locale_core/2.4.0) | `Unicode-3.0` | 1.88 |
| [icu_normalizer 2.4.0](https://crates.io/crates/icu_normalizer/2.4.0) | `Unicode-3.0` | 1.88 |
| [icu_normalizer_data 2.4.0](https://crates.io/crates/icu_normalizer_data/2.4.0) | `Unicode-3.0` | 1.88 |
| [icu_properties 2.4.0](https://crates.io/crates/icu_properties/2.4.0) | `Unicode-3.0` | 1.88 |
| [icu_properties_data 2.4.0](https://crates.io/crates/icu_properties_data/2.4.0) | `Unicode-3.0` | 1.88 |
| [icu_provider 2.4.0](https://crates.io/crates/icu_provider/2.4.0) | `Unicode-3.0` | 1.88 |
| [ident_case 1.0.1](https://crates.io/crates/ident_case/1.0.1) | `MIT/Apache-2.0` | não declarado |
| [idna 1.1.0](https://crates.io/crates/idna/1.1.0) | `MIT OR Apache-2.0` | 1.57 |
| [idna_adapter 1.2.2](https://crates.io/crates/idna_adapter/1.2.2) | `Apache-2.0 OR MIT` | 1.86 |
| [image 0.25.10](https://crates.io/crates/image/0.25.10) | `MIT OR Apache-2.0` | 1.88.0 |
| [image-webp 0.2.4](https://crates.io/crates/image-webp/0.2.4) | `MIT OR Apache-2.0` | 1.80.1 |
| [indexmap 2.14.2](https://crates.io/crates/indexmap/2.14.2) | `Apache-2.0 OR MIT` | 1.85 |
| [inotify 0.11.5](https://crates.io/crates/inotify/0.11.5) | `ISC` | 1.70 |
| [inotify-sys 0.1.8](https://crates.io/crates/inotify-sys/0.1.8) | `ISC` | não declarado |
| [inout 0.1.4](https://crates.io/crates/inout/0.1.4) | `MIT OR Apache-2.0` | 1.56 |
| [ipconfig 0.3.4](https://crates.io/crates/ipconfig/0.3.4) | `MIT/Apache-2.0` | 1.71 |
| [ipnet 2.12.2](https://crates.io/crates/ipnet/2.12.2) | `MIT OR Apache-2.0` | não declarado |
| [is-docker 0.2.0](https://crates.io/crates/is-docker/0.2.0) | `MIT` | não declarado |
| [is-wsl 0.4.0](https://crates.io/crates/is-wsl/0.4.0) | `MIT` | não declarado |
| [itertools 0.15.0](https://crates.io/crates/itertools/0.15.0) | `MIT OR Apache-2.0` | 1.63.0 |
| [itoa 1.0.18](https://crates.io/crates/itoa/1.0.18) | `MIT OR Apache-2.0` | 1.68 |
| [jni 0.22.4](https://crates.io/crates/jni/0.22.4) | `MIT OR Apache-2.0` | 1.85.0 |
| [jni-macros 0.22.4](https://crates.io/crates/jni-macros/0.22.4) | `MIT OR Apache-2.0` | não declarado |
| [jni-sys 0.3.1](https://crates.io/crates/jni-sys/0.3.1) | `MIT OR Apache-2.0` | 1.77.0 |
| [jni-sys 0.4.1](https://crates.io/crates/jni-sys/0.4.1) | `MIT OR Apache-2.0` | 1.76.0 |
| [jni-sys-macros 0.4.1](https://crates.io/crates/jni-sys-macros/0.4.1) | `MIT OR Apache-2.0` | não declarado |
| [jobserver 0.1.35](https://crates.io/crates/jobserver/0.1.35) | `MIT OR Apache-2.0` | 1.85 |
| [js-sys 0.3.106](https://crates.io/crates/js-sys/0.3.106) | `MIT OR Apache-2.0` | 1.81 |
| [keyring 3.6.3](https://crates.io/crates/keyring/3.6.3) | `MIT OR Apache-2.0` | 1.75 |
| [khronos-egl 6.0.0](https://crates.io/crates/khronos-egl/6.0.0) | `MIT/Apache-2.0` | não declarado |
| [khronos_api 3.1.0](https://crates.io/crates/khronos_api/3.1.0) | `Apache-2.0` | não declarado |
| [kittest 0.5.0](https://crates.io/crates/kittest/0.5.0) | `MIT OR Apache-2.0` | 1.95 |
| [kqueue 1.2.1](https://crates.io/crates/kqueue/1.2.1) | `MIT` | não declarado |
| [kqueue-sys 1.1.2](https://crates.io/crates/kqueue-sys/1.1.2) | `MIT` | 1.65 |
| [kurbo 0.13.1](https://crates.io/crates/kurbo/0.13.1) | `Apache-2.0 OR MIT` | 1.85 |
| [lazy_static 1.5.1](https://crates.io/crates/lazy_static/1.5.1) | `MIT OR Apache-2.0` | não declarado |
| [libc 0.2.191](https://crates.io/crates/libc/0.2.191) | `MIT OR Apache-2.0` | 1.65 |
| [libdbus-sys 0.2.7](https://crates.io/crates/libdbus-sys/0.2.7) | `Apache-2.0/MIT` | não declarado |
| [libloading 0.8.9](https://crates.io/crates/libloading/0.8.9) | `ISC` | 1.71.0 |
| [libm 0.2.16](https://crates.io/crates/libm/0.2.16) | `MIT` | 1.63 |
| [libredox 0.1.25](https://crates.io/crates/libredox/0.1.25) | `MIT` | não declarado |
| [libsqlite3-sys 0.38.2](https://crates.io/crates/libsqlite3-sys/0.38.2) | `MIT` | não declarado |
| [linebender_resource_handle 0.1.1](https://crates.io/crates/linebender_resource_handle/0.1.1) | `Apache-2.0 OR MIT` | 1.70 |
| [linux-raw-sys 0.12.1](https://crates.io/crates/linux-raw-sys/0.12.1) | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 1.63 |
| [linux-raw-sys 0.4.15](https://crates.io/crates/linux-raw-sys/0.4.15) | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 1.63 |
| [litemap 0.8.3](https://crates.io/crates/litemap/0.8.3) | `Unicode-3.0` | 1.82 |
| [litrs 1.0.1](https://crates.io/crates/litrs/1.0.1) | `MIT OR Apache-2.0` | 1.56 |
| [lock_api 0.4.14](https://crates.io/crates/lock_api/0.4.14) | `MIT OR Apache-2.0` | 1.71.0 |
| [log 0.4.34](https://crates.io/crates/log/0.4.34) | `MIT OR Apache-2.0` | 1.71.0 |
| [lru-slab 0.1.3](https://crates.io/crates/lru-slab/0.1.3) | `MIT OR Apache-2.0 OR Zlib` | não declarado |
| [macro_magic 0.5.1](https://crates.io/crates/macro_magic/0.5.1) | `MIT` | não declarado |
| [macro_magic_core 0.5.1](https://crates.io/crates/macro_magic_core/0.5.1) | `MIT` | não declarado |
| [macro_magic_core_macros 0.5.1](https://crates.io/crates/macro_magic_core_macros/0.5.1) | `MIT` | não declarado |
| [macro_magic_macros 0.5.1](https://crates.io/crates/macro_magic_macros/0.5.1) | `MIT` | não declarado |
| [md-5 0.10.6](https://crates.io/crates/md-5/0.10.6) | `MIT OR Apache-2.0` | não declarado |
| [memchr 2.8.3](https://crates.io/crates/memchr/2.8.3) | `Unlicense OR MIT` | 1.61 |
| [memmap2 0.9.11](https://crates.io/crates/memmap2/0.9.11) | `MIT OR Apache-2.0` | 1.65 |
| [memoffset 0.9.1](https://crates.io/crates/memoffset/0.9.1) | `MIT` | não declarado |
| [mime 0.3.17](https://crates.io/crates/mime/0.3.17) | `MIT OR Apache-2.0` | não declarado |
| [mime_guess 2.0.5](https://crates.io/crates/mime_guess/2.0.5) | `MIT` | não declarado |
| [miniz_oxide 0.8.9](https://crates.io/crates/miniz_oxide/0.8.9) | `MIT OR Zlib OR Apache-2.0` | não declarado |
| [miniz_oxide 0.9.1](https://crates.io/crates/miniz_oxide/0.9.1) | `MIT OR Zlib OR Apache-2.0` | não declarado |
| [mio 1.2.4](https://crates.io/crates/mio/1.2.4) | `MIT` | 1.71 |
| [moka 0.12.16](https://crates.io/crates/moka/0.12.16) | `(MIT OR Apache-2.0) AND Apache-2.0` | 1.71.1 |
| [mongocrypt 0.3.2](https://crates.io/crates/mongocrypt/0.3.2) | `Apache-2.0` | não declarado |
| [mongocrypt-sys 0.1.6+1.18.2](https://crates.io/crates/mongocrypt-sys/0.1.6+1.18.2) | `Apache-2.0` | não declarado |
| [mongodb 3.7.0](https://crates.io/crates/mongodb/3.7.0) | `Apache-2.0` | 1.88 |
| [mongodb-internal-macros 3.9.1](https://crates.io/crates/mongodb-internal-macros/3.9.1) | `Apache-2.0` | 1.88 |
| [moxcms 0.8.1](https://crates.io/crates/moxcms/0.8.1) | `BSD-3-Clause OR Apache-2.0` | 1.85.0 |
| [naga 30.0.1](https://crates.io/crates/naga/30.0.1) | `MIT OR Apache-2.0` | 1.87 |
| [naga-types 30.0.1](https://crates.io/crates/naga-types/30.0.1) | `MIT OR Apache-2.0` | não declarado |
| [ndk 0.9.0](https://crates.io/crates/ndk/0.9.0) | `MIT OR Apache-2.0` | 1.66 |
| [ndk-context 0.1.1](https://crates.io/crates/ndk-context/0.1.1) | `MIT OR Apache-2.0` | não declarado |
| [ndk-sys 0.6.0+11769913](https://crates.io/crates/ndk-sys/0.6.0+11769913) | `MIT OR Apache-2.0` | 1.60 |
| [nix 0.28.0](https://crates.io/crates/nix/0.28.0) | `MIT` | 1.69 |
| [nix 0.29.0](https://crates.io/crates/nix/0.29.0) | `MIT` | 1.69 |
| [nix 0.31.4](https://crates.io/crates/nix/0.31.4) | `MIT` | 1.69 |
| [nohash-hasher 0.2.0](https://crates.io/crates/nohash-hasher/0.2.0) | `Apache-2.0 OR MIT` | não declarado |
| [nom 8.0.0](https://crates.io/crates/nom/8.0.0) | `MIT` | 1.65.0 |
| [notify 8.2.1](https://crates.io/crates/notify/8.2.1) | `CC0-1.0` | 1.85 |
| [notify-types 2.1.0](https://crates.io/crates/notify-types/2.1.0) | `MIT OR Apache-2.0` | 1.85 |
| [num 0.4.3](https://crates.io/crates/num/0.4.3) | `MIT OR Apache-2.0` | 1.60 |
| [num-bigint 0.4.8](https://crates.io/crates/num-bigint/0.4.8) | `MIT OR Apache-2.0` | 1.60 |
| [num-bigint-dig 0.8.6](https://crates.io/crates/num-bigint-dig/0.8.6) | `MIT/Apache-2.0` | 1.56 |
| [num-complex 0.4.6](https://crates.io/crates/num-complex/0.4.6) | `MIT OR Apache-2.0` | 1.60 |
| [num-conv 0.2.2](https://crates.io/crates/num-conv/0.2.2) | `MIT OR Apache-2.0` | 1.57.0 |
| [num-integer 0.1.47](https://crates.io/crates/num-integer/0.1.47) | `MIT OR Apache-2.0` | 1.31 |
| [num-iter 0.1.46](https://crates.io/crates/num-iter/0.1.46) | `MIT OR Apache-2.0` | 1.31 |
| [num-rational 0.4.2](https://crates.io/crates/num-rational/0.4.2) | `MIT OR Apache-2.0` | 1.60 |
| [num-traits 0.2.19](https://crates.io/crates/num-traits/0.2.19) | `MIT OR Apache-2.0` | 1.60 |
| [num_enum 0.7.6](https://crates.io/crates/num_enum/0.7.6) | `BSD-3-Clause OR MIT OR Apache-2.0` | 1.70.0 |
| [num_enum_derive 0.7.6](https://crates.io/crates/num_enum_derive/0.7.6) | `BSD-3-Clause OR MIT OR Apache-2.0` | 1.70.0 |
| [objc-sys 0.3.5](https://crates.io/crates/objc-sys/0.3.5) | `MIT` | 1.60 |
| [objc2 0.5.3](https://crates.io/crates/objc2/0.5.3) | `MIT` | 1.60 |
| [objc2 0.6.5](https://crates.io/crates/objc2/0.6.5) | `MIT` | 1.71 |
| [objc2-app-kit 0.2.2](https://crates.io/crates/objc2-app-kit/0.2.2) | `MIT` | 1.60 |
| [objc2-app-kit 0.3.2](https://crates.io/crates/objc2-app-kit/0.3.2) | `Zlib OR Apache-2.0 OR MIT` | 1.71 |
| [objc2-cloud-kit 0.2.2](https://crates.io/crates/objc2-cloud-kit/0.2.2) | `MIT` | 1.60 |
| [objc2-contacts 0.2.2](https://crates.io/crates/objc2-contacts/0.2.2) | `MIT` | 1.60 |
| [objc2-core-data 0.2.2](https://crates.io/crates/objc2-core-data/0.2.2) | `MIT` | 1.60 |
| [objc2-core-foundation 0.3.2](https://crates.io/crates/objc2-core-foundation/0.3.2) | `Zlib OR Apache-2.0 OR MIT` | 1.71 |
| [objc2-core-graphics 0.3.2](https://crates.io/crates/objc2-core-graphics/0.3.2) | `Zlib OR Apache-2.0 OR MIT` | 1.71 |
| [objc2-core-image 0.2.2](https://crates.io/crates/objc2-core-image/0.2.2) | `MIT` | 1.60 |
| [objc2-core-location 0.2.2](https://crates.io/crates/objc2-core-location/0.2.2) | `MIT` | 1.60 |
| [objc2-core-services 0.3.2](https://crates.io/crates/objc2-core-services/0.3.2) | `Zlib OR Apache-2.0 OR MIT` | 1.71 |
| [objc2-encode 4.1.0](https://crates.io/crates/objc2-encode/4.1.0) | `MIT` | 1.71 |
| [objc2-foundation 0.2.2](https://crates.io/crates/objc2-foundation/0.2.2) | `MIT` | 1.60 |
| [objc2-foundation 0.3.2](https://crates.io/crates/objc2-foundation/0.3.2) | `MIT` | 1.71 |
| [objc2-io-surface 0.3.2](https://crates.io/crates/objc2-io-surface/0.3.2) | `Zlib OR Apache-2.0 OR MIT` | 1.71 |
| [objc2-link-presentation 0.2.2](https://crates.io/crates/objc2-link-presentation/0.2.2) | `MIT` | 1.60 |
| [objc2-metal 0.2.2](https://crates.io/crates/objc2-metal/0.2.2) | `MIT` | 1.60 |
| [objc2-metal 0.3.2](https://crates.io/crates/objc2-metal/0.3.2) | `Zlib OR Apache-2.0 OR MIT` | 1.71 |
| [objc2-quartz-core 0.2.2](https://crates.io/crates/objc2-quartz-core/0.2.2) | `MIT` | 1.60 |
| [objc2-quartz-core 0.3.2](https://crates.io/crates/objc2-quartz-core/0.3.2) | `Zlib OR Apache-2.0 OR MIT` | 1.71 |
| [objc2-symbols 0.2.2](https://crates.io/crates/objc2-symbols/0.2.2) | `MIT` | 1.60 |
| [objc2-ui-kit 0.2.2](https://crates.io/crates/objc2-ui-kit/0.2.2) | `MIT` | 1.60 |
| [objc2-ui-kit 0.3.2](https://crates.io/crates/objc2-ui-kit/0.3.2) | `Zlib OR Apache-2.0 OR MIT` | 1.71 |
| [objc2-uniform-type-identifiers 0.2.2](https://crates.io/crates/objc2-uniform-type-identifiers/0.2.2) | `MIT` | 1.60 |
| [objc2-user-notifications 0.2.2](https://crates.io/crates/objc2-user-notifications/0.2.2) | `MIT` | 1.60 |
| [object 0.39.1](https://crates.io/crates/object/0.39.1) | `Apache-2.0 OR MIT` | 1.65 |
| [once_cell 1.21.4](https://crates.io/crates/once_cell/1.21.4) | `MIT OR Apache-2.0` | 1.65 |
| [open 5.4.4](https://crates.io/crates/open/5.4.4) | `MIT` | 1.62 |
| [openssl-probe 0.2.1](https://crates.io/crates/openssl-probe/0.2.1) | `MIT OR Apache-2.0` | 1.60.0 |
| [orbclient 0.3.55](https://crates.io/crates/orbclient/0.3.55) | `MIT` | não declarado |
| [ordered-float 5.5.0](https://crates.io/crates/ordered-float/5.5.0) | `MIT` | 1.90 |
| [ordered-stream 0.2.0](https://crates.io/crates/ordered-stream/0.2.0) | `MIT OR Apache-2.0` | não declarado |
| [os_pipe 1.2.3](https://crates.io/crates/os_pipe/1.2.3) | `MIT` | 1.63 |
| [parking 2.2.1](https://crates.io/crates/parking/2.2.1) | `Apache-2.0 OR MIT` | 1.51 |
| [parking_lot 0.12.5](https://crates.io/crates/parking_lot/0.12.5) | `MIT OR Apache-2.0` | 1.71 |
| [parking_lot_core 0.9.12](https://crates.io/crates/parking_lot_core/0.9.12) | `MIT OR Apache-2.0` | 1.71.0 |
| [pbkdf2 0.12.2](https://crates.io/crates/pbkdf2/0.12.2) | `MIT OR Apache-2.0` | 1.60 |
| [pem-rfc7468 0.7.0](https://crates.io/crates/pem-rfc7468/0.7.0) | `Apache-2.0 OR MIT` | 1.60 |
| [peniko 0.6.1](https://crates.io/crates/peniko/0.6.1) | `Apache-2.0 OR MIT` | 1.85 |
| [percent-encoding 2.3.2](https://crates.io/crates/percent-encoding/2.3.2) | `MIT OR Apache-2.0` | 1.51 |
| [petgraph 0.8.3](https://crates.io/crates/petgraph/0.8.3) | `MIT OR Apache-2.0` | 1.64 |
| [phf 0.14.0](https://crates.io/crates/phf/0.14.0) | `MIT` | 1.85 |
| [phf_generator 0.14.0](https://crates.io/crates/phf_generator/0.14.0) | `MIT` | 1.85 |
| [phf_macros 0.14.0](https://crates.io/crates/phf_macros/0.14.0) | `MIT` | 1.85 |
| [phf_shared 0.14.0](https://crates.io/crates/phf_shared/0.14.0) | `MIT` | 1.85 |
| [pin-project 1.1.13](https://crates.io/crates/pin-project/1.1.13) | `Apache-2.0 OR MIT` | 1.71 |
| [pin-project-internal 1.1.13](https://crates.io/crates/pin-project-internal/1.1.13) | `Apache-2.0 OR MIT` | 1.71 |
| [pin-project-lite 0.2.17](https://crates.io/crates/pin-project-lite/0.2.17) | `Apache-2.0 OR MIT` | 1.37 |
| [piper 0.2.5](https://crates.io/crates/piper/0.2.5) | `MIT OR Apache-2.0` | 1.36 |
| [pkcs1 0.7.5](https://crates.io/crates/pkcs1/0.7.5) | `Apache-2.0 OR MIT` | 1.60 |
| [pkcs8 0.10.2](https://crates.io/crates/pkcs8/0.10.2) | `Apache-2.0 OR MIT` | 1.65 |
| [pkg-config 0.3.34](https://crates.io/crates/pkg-config/0.3.34) | `MIT OR Apache-2.0` | 1.63 |
| [plain 0.2.3](https://crates.io/crates/plain/0.2.3) | `MIT/Apache-2.0` | não declarado |
| [png 0.18.1](https://crates.io/crates/png/0.18.1) | `MIT OR Apache-2.0` | 1.73 |
| [polling 3.11.0](https://crates.io/crates/polling/3.11.0) | `Apache-2.0 OR MIT` | 1.71 |
| [pollster 0.4.0](https://crates.io/crates/pollster/0.4.0) | `Apache-2.0/MIT` | não declarado |
| [pollster 1.0.1](https://crates.io/crates/pollster/1.0.1) | `Apache-2.0/MIT` | 1.69 |
| [polycool 0.4.0](https://crates.io/crates/polycool/0.4.0) | `MIT OR Apache-2.0` | não declarado |
| [portable-atomic 1.15.0](https://crates.io/crates/portable-atomic/1.15.0) | `Apache-2.0 OR MIT` | 1.34 |
| [portable-atomic-util 0.2.8](https://crates.io/crates/portable-atomic-util/0.2.8) | `Apache-2.0 OR MIT` | 1.36 |
| [portable-pty 0.9.0](https://crates.io/crates/portable-pty/0.9.0) | `MIT` | não declarado |
| [potential_utf 0.1.6](https://crates.io/crates/potential_utf/0.1.6) | `Unicode-3.0` | 1.82 |
| [powerfmt 0.2.1](https://crates.io/crates/powerfmt/0.2.1) | `MIT OR Apache-2.0` | 1.79.0 |
| [ppv-lite86 0.2.21](https://crates.io/crates/ppv-lite86/0.2.21) | `MIT OR Apache-2.0` | 1.61 |
| [prefix-trie 0.8.4](https://crates.io/crates/prefix-trie/0.8.4) | `MIT OR Apache-2.0` | 1.71.1 |
| [presser 0.3.1](https://crates.io/crates/presser/0.3.1) | `MIT OR Apache-2.0` | 1.64 |
| [proc-macro-crate 3.5.0](https://crates.io/crates/proc-macro-crate/3.5.0) | `MIT OR Apache-2.0` | 1.82.0 |
| [proc-macro2 1.0.107](https://crates.io/crates/proc-macro2/1.0.107) | `MIT OR Apache-2.0` | 1.71 |
| [profiling 1.0.18](https://crates.io/crates/profiling/1.0.18) | `MIT OR Apache-2.0` | 1.60 |
| [psl-types 2.0.11](https://crates.io/crates/psl-types/2.0.11) | `MIT/Apache-2.0` | não declarado |
| [psm 0.1.32](https://crates.io/crates/psm/0.1.32) | `MIT OR Apache-2.0` | 1.88.0 |
| [publicsuffix 2.3.0](https://crates.io/crates/publicsuffix/2.3.0) | `MIT/Apache-2.0` | não declarado |
| [pulldown-cmark 0.13.4](https://crates.io/crates/pulldown-cmark/0.13.4) | `MIT` | 1.71.1 |
| [pxfm 0.1.30](https://crates.io/crates/pxfm/0.1.30) | `BSD-3-Clause OR Apache-2.0` | 1.85 |
| [quick-error 2.0.1](https://crates.io/crates/quick-error/2.0.1) | `MIT/Apache-2.0` | não declarado |
| [quick-xml 0.41.0](https://crates.io/crates/quick-xml/0.41.0) | `MIT` | 1.79 |
| [quinn 0.11.12](https://crates.io/crates/quinn/0.11.12) | `MIT OR Apache-2.0` | 1.85 |
| [quinn-proto 0.11.19](https://crates.io/crates/quinn-proto/0.11.19) | `MIT OR Apache-2.0` | 1.85 |
| [quinn-udp 0.5.16](https://crates.io/crates/quinn-udp/0.5.16) | `MIT OR Apache-2.0` | 1.85 |
| [quote 1.0.47](https://crates.io/crates/quote/1.0.47) | `MIT OR Apache-2.0` | 1.71 |
| [r-efi 5.3.0](https://crates.io/crates/r-efi/5.3.0) | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | 1.68 |
| [r-efi 6.0.0](https://crates.io/crates/r-efi/6.0.0) | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | 1.68 |
| [radium 0.7.0](https://crates.io/crates/radium/0.7.0) | `MIT` | não declarado |
| [rand 0.10.3](https://crates.io/crates/rand/0.10.3) | `MIT OR Apache-2.0` | 1.85 |
| [rand 0.8.8](https://crates.io/crates/rand/0.8.8) | `MIT OR Apache-2.0` | não declarado |
| [rand 0.9.5](https://crates.io/crates/rand/0.9.5) | `MIT OR Apache-2.0` | 1.63 |
| [rand_chacha 0.3.1](https://crates.io/crates/rand_chacha/0.3.1) | `MIT OR Apache-2.0` | não declarado |
| [rand_chacha 0.9.0](https://crates.io/crates/rand_chacha/0.9.0) | `MIT OR Apache-2.0` | 1.63 |
| [rand_core 0.10.1](https://crates.io/crates/rand_core/0.10.1) | `MIT OR Apache-2.0` | 1.85 |
| [rand_core 0.6.4](https://crates.io/crates/rand_core/0.6.4) | `MIT OR Apache-2.0` | não declarado |
| [rand_core 0.9.5](https://crates.io/crates/rand_core/0.9.5) | `MIT OR Apache-2.0` | 1.63 |
| [rand_pcg 0.10.2](https://crates.io/crates/rand_pcg/0.10.2) | `MIT OR Apache-2.0` | 1.85 |
| [range-alloc 0.1.5](https://crates.io/crates/range-alloc/0.1.5) | `MIT OR Apache-2.0` | não declarado |
| [raw-window-handle 0.6.2](https://crates.io/crates/raw-window-handle/0.6.2) | `MIT OR Apache-2.0 OR Zlib` | 1.64 |
| [raw-window-metal 1.1.0](https://crates.io/crates/raw-window-metal/1.1.0) | `MIT OR Apache-2.0` | 1.71 |
| [rayon 1.12.0](https://crates.io/crates/rayon/1.12.0) | `MIT OR Apache-2.0` | 1.80 |
| [rayon-core 1.13.0](https://crates.io/crates/rayon-core/1.13.0) | `MIT OR Apache-2.0` | 1.80 |
| [read-fonts 0.41.0](https://crates.io/crates/read-fonts/0.41.0) | `MIT OR Apache-2.0` | 1.85 |
| [recursive 0.1.1](https://crates.io/crates/recursive/0.1.1) | `MIT` | não declarado |
| [recursive-proc-macro-impl 0.1.1](https://crates.io/crates/recursive-proc-macro-impl/0.1.1) | `MIT` | não declarado |
| [redis 0.32.7](https://crates.io/crates/redis/0.32.7) | `BSD-3-Clause` | 1.80 |
| [redox_syscall 0.4.1](https://crates.io/crates/redox_syscall/0.4.1) | `MIT` | não declarado |
| [redox_syscall 0.5.18](https://crates.io/crates/redox_syscall/0.5.18) | `MIT` | não declarado |
| [redox_syscall 0.9.5](https://crates.io/crates/redox_syscall/0.9.5) | `MIT` | não declarado |
| [regex 1.13.1](https://crates.io/crates/regex/1.13.1) | `MIT OR Apache-2.0` | 1.65 |
| [regex-automata 0.4.18](https://crates.io/crates/regex-automata/0.4.18) | `MIT OR Apache-2.0` | 1.65 |
| [regex-syntax 0.8.11](https://crates.io/crates/regex-syntax/0.8.11) | `MIT OR Apache-2.0` | 1.65 |
| [renderdoc-sys 1.1.0](https://crates.io/crates/renderdoc-sys/1.1.0) | `MIT OR Apache-2.0` | não declarado |
| [reqwest 0.12.28](https://crates.io/crates/reqwest/0.12.28) | `MIT OR Apache-2.0` | 1.64.0 |
| [resolv-conf 0.7.6](https://crates.io/crates/resolv-conf/0.7.6) | `MIT OR Apache-2.0` | 1.61 |
| [rfd 0.17.2](https://crates.io/crates/rfd/0.17.2) | `MIT` | não declarado |
| [ring 0.17.14](https://crates.io/crates/ring/0.17.14) | `Apache-2.0 AND ISC` | 1.66.0 |
| [ropey 1.6.1](https://crates.io/crates/ropey/1.6.1) | `MIT` | não declarado |
| [rsa 0.9.10](https://crates.io/crates/rsa/0.9.10) | `MIT OR Apache-2.0` | 1.65 |
| [rusqlite 0.40.2](https://crates.io/crates/rusqlite/0.40.2) | `MIT` | não declarado |
| [rustc-hash 1.1.0](https://crates.io/crates/rustc-hash/1.1.0) | `Apache-2.0/MIT` | não declarado |
| [rustc-hash 2.1.3](https://crates.io/crates/rustc-hash/2.1.3) | `Apache-2.0 OR MIT` | 1.77 |
| [rustc_version 0.4.1](https://crates.io/crates/rustc_version/0.4.1) | `MIT OR Apache-2.0` | 1.32 |
| [rustc_version_runtime 0.3.0](https://crates.io/crates/rustc_version_runtime/0.3.0) | `MIT` | não declarado |
| [rustix 0.38.44](https://crates.io/crates/rustix/0.38.44) | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 1.63 |
| [rustix 1.1.5](https://crates.io/crates/rustix/1.1.5) | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 1.65 |
| [rustls 0.23.46](https://crates.io/crates/rustls/0.23.46) | `Apache-2.0 OR ISC OR MIT` | 1.71 |
| [rustls-native-certs 0.8.5](https://crates.io/crates/rustls-native-certs/0.8.5) | `Apache-2.0 OR ISC OR MIT` | 1.71 |
| [rustls-pki-types 1.15.1](https://crates.io/crates/rustls-pki-types/1.15.1) | `MIT OR Apache-2.0` | 1.60 |
| [rustls-webpki 0.103.16](https://crates.io/crates/rustls-webpki/0.103.16) | `ISC` | 1.71 |
| [rustversion 1.0.23](https://crates.io/crates/rustversion/1.0.23) | `MIT OR Apache-2.0` | 1.31 |
| [ryu 1.0.23](https://crates.io/crates/ryu/1.0.23) | `Apache-2.0 OR BSL-1.0` | 1.71 |
| [same-file 1.0.6](https://crates.io/crates/same-file/1.0.6) | `Unlicense/MIT` | não declarado |
| [schannel 0.1.30](https://crates.io/crates/schannel/0.1.30) | `MIT` | 1.71.0 |
| [scoped-tls 1.0.1](https://crates.io/crates/scoped-tls/1.0.1) | `MIT/Apache-2.0` | 1.59 |
| [scopeguard 1.2.0](https://crates.io/crates/scopeguard/1.2.0) | `MIT OR Apache-2.0` | não declarado |
| [secret-service 4.0.0](https://crates.io/crates/secret-service/4.0.0) | `MIT OR Apache-2.0` | 1.75.0 |
| [security-framework 2.11.1](https://crates.io/crates/security-framework/2.11.1) | `MIT OR Apache-2.0` | 1.60 |
| [security-framework 3.7.0](https://crates.io/crates/security-framework/3.7.0) | `MIT OR Apache-2.0` | 1.85 |
| [security-framework-sys 2.17.0](https://crates.io/crates/security-framework-sys/2.17.0) | `MIT OR Apache-2.0` | 1.70 |
| [self_cell 1.3.0](https://crates.io/crates/self_cell/1.3.0) | `Apache-2.0 OR GPL-2.0-only` | não declarado |
| [semver 1.0.28](https://crates.io/crates/semver/1.0.28) | `MIT OR Apache-2.0` | 1.68 |
| [send_wrapper 0.6.0](https://crates.io/crates/send_wrapper/0.6.0) | `MIT/Apache-2.0` | não declarado |
| [serde 1.0.229](https://crates.io/crates/serde/1.0.229) | `MIT OR Apache-2.0` | 1.56 |
| [serde_bytes 0.11.19](https://crates.io/crates/serde_bytes/0.11.19) | `MIT OR Apache-2.0` | 1.68 |
| [serde_core 1.0.229](https://crates.io/crates/serde_core/1.0.229) | `MIT OR Apache-2.0` | 1.56 |
| [serde_derive 1.0.229](https://crates.io/crates/serde_derive/1.0.229) | `MIT OR Apache-2.0` | 1.71 |
| [serde_json 1.0.154](https://crates.io/crates/serde_json/1.0.154) | `MIT OR Apache-2.0` | 1.71 |
| [serde_repr 0.1.21](https://crates.io/crates/serde_repr/0.1.21) | `MIT OR Apache-2.0` | 1.71 |
| [serde_spanned 1.1.2](https://crates.io/crates/serde_spanned/1.1.2) | `MIT OR Apache-2.0` | 1.85 |
| [serde_urlencoded 0.7.1](https://crates.io/crates/serde_urlencoded/0.7.1) | `MIT/Apache-2.0` | não declarado |
| [serde_with 3.24.0](https://crates.io/crates/serde_with/3.24.0) | `MIT OR Apache-2.0` | 1.88 |
| [serde_with_macros 3.24.0](https://crates.io/crates/serde_with_macros/3.24.0) | `MIT OR Apache-2.0` | 1.88 |
| [serde_yaml 0.9.34+deprecated](https://crates.io/crates/serde_yaml/0.9.34+deprecated) | `MIT OR Apache-2.0` | 1.64 |
| [serial2 0.2.38](https://crates.io/crates/serial2/0.2.38) | `BSD-2-Clause OR Apache-2.0` | 1.63 |
| [sha1 0.10.7](https://crates.io/crates/sha1/0.10.7) | `MIT OR Apache-2.0` | não declarado |
| [sha1 0.11.0](https://crates.io/crates/sha1/0.11.0) | `MIT OR Apache-2.0` | 1.85 |
| [sha1_smol 1.0.1](https://crates.io/crates/sha1_smol/1.0.1) | `BSD-3-Clause` | não declarado |
| [sha2 0.10.9](https://crates.io/crates/sha2/0.10.9) | `MIT OR Apache-2.0` | não declarado |
| [shared_library 0.1.9](https://crates.io/crates/shared_library/0.1.9) | `Apache-2.0/MIT` | não declarado |
| [shell-words 1.1.1](https://crates.io/crates/shell-words/1.1.1) | `MIT/Apache-2.0` | não declarado |
| [shlex 2.0.1](https://crates.io/crates/shlex/2.0.1) | `MIT OR Apache-2.0` | 1.46.0 |
| [signal-hook-registry 1.4.8](https://crates.io/crates/signal-hook-registry/1.4.8) | `MIT OR Apache-2.0` | 1.26 |
| [signature 2.2.0](https://crates.io/crates/signature/2.2.0) | `Apache-2.0 OR MIT` | 1.60 |
| [simd-adler32 0.3.10](https://crates.io/crates/simd-adler32/0.3.10) | `MIT` | não declarado |
| [simd_cesu8 1.2.0](https://crates.io/crates/simd_cesu8/1.2.0) | `Apache-2.0 OR MIT` | 1.85.0 |
| [simdutf8 0.1.5](https://crates.io/crates/simdutf8/0.1.5) | `MIT OR Apache-2.0` | não declarado |
| [siphasher 1.0.4](https://crates.io/crates/siphasher/1.0.4) | `MIT OR Apache-2.0` | não declarado |
| [skrifa 0.44.0](https://crates.io/crates/skrifa/0.44.0) | `MIT OR Apache-2.0` | 1.85 |
| [slab 0.4.12](https://crates.io/crates/slab/0.4.12) | `MIT` | 1.51 |
| [slotmap 1.1.1](https://crates.io/crates/slotmap/1.1.1) | `Zlib` | 1.58.0 |
| [smallvec 1.16.3](https://crates.io/crates/smallvec/1.16.3) | `MIT OR Apache-2.0` | não declarado |
| [smithay-client-toolkit 0.19.2](https://crates.io/crates/smithay-client-toolkit/0.19.2) | `MIT` | não declarado |
| [smithay-client-toolkit 0.20.0](https://crates.io/crates/smithay-client-toolkit/0.20.0) | `MIT` | 1.65 |
| [smithay-clipboard 0.7.3](https://crates.io/crates/smithay-clipboard/0.7.3) | `MIT` | 1.85.0 |
| [smol_str 0.2.2](https://crates.io/crates/smol_str/0.2.2) | `MIT OR Apache-2.0` | não declarado |
| [socket2 0.6.5](https://crates.io/crates/socket2/0.6.5) | `MIT OR Apache-2.0` | 1.70 |
| [spin 0.9.9](https://crates.io/crates/spin/0.9.9) | `MIT` | 1.38 |
| [spirv 0.4.0+sdk-1.4.341.0](https://crates.io/crates/spirv/0.4.0+sdk-1.4.341.0) | `Apache-2.0` | não declarado |
| [spki 0.7.3](https://crates.io/crates/spki/0.7.3) | `Apache-2.0 OR MIT` | 1.65 |
| [sqlformat 0.5.0](https://crates.io/crates/sqlformat/0.5.0) | `MIT OR Apache-2.0` | 1.84 |
| [sqlparser 0.59.0](https://crates.io/crates/sqlparser/0.59.0) | `Apache-2.0` | não declarado |
| [sqlx-core 0.8.6](https://crates.io/crates/sqlx-core/0.8.6) | `MIT OR Apache-2.0` | não declarado |
| [sqlx-mysql 0.8.6](https://crates.io/crates/sqlx-mysql/0.8.6) | `MIT OR Apache-2.0` | não declarado |
| [sqlx-postgres 0.8.6](https://crates.io/crates/sqlx-postgres/0.8.6) | `MIT OR Apache-2.0` | não declarado |
| [stable_deref_trait 1.2.1](https://crates.io/crates/stable_deref_trait/1.2.1) | `MIT OR Apache-2.0` | não declarado |
| [stacker 0.1.25](https://crates.io/crates/stacker/0.1.25) | `MIT OR Apache-2.0` | 1.63 |
| [static_assertions 1.1.0](https://crates.io/crates/static_assertions/1.1.0) | `MIT OR Apache-2.0` | não declarado |
| [str_indices 0.4.4](https://crates.io/crates/str_indices/0.4.4) | `MIT OR Apache-2.0` | não declarado |
| [streaming-iterator 0.1.9](https://crates.io/crates/streaming-iterator/0.1.9) | `MIT OR Apache-2.0` | 1.56 |
| [stringprep 0.1.5](https://crates.io/crates/stringprep/0.1.5) | `MIT/Apache-2.0` | não declarado |
| [strsim 0.11.1](https://crates.io/crates/strsim/0.11.1) | `MIT` | 1.56 |
| [subtle 2.6.1](https://crates.io/crates/subtle/2.6.1) | `BSD-3-Clause` | não declarado |
| [syn 2.0.119](https://crates.io/crates/syn/2.0.119) | `MIT OR Apache-2.0` | 1.71 |
| [syn 3.0.9](https://crates.io/crates/syn/3.0.9) | `MIT OR Apache-2.0` | 1.71 |
| [sync_wrapper 1.0.2](https://crates.io/crates/sync_wrapper/1.0.2) | `Apache-2.0` | não declarado |
| [synstructure 0.14.0](https://crates.io/crates/synstructure/0.14.0) | `MIT` | 1.71 |
| [system-configuration 0.7.0](https://crates.io/crates/system-configuration/0.7.0) | `MIT OR Apache-2.0` | 1.64.0 |
| [system-configuration-sys 0.6.0](https://crates.io/crates/system-configuration-sys/0.6.0) | `MIT OR Apache-2.0` | não declarado |
| [tagptr 0.2.0](https://crates.io/crates/tagptr/0.2.0) | `MIT/Apache-2.0` | não declarado |
| [take_mut 0.2.2](https://crates.io/crates/take_mut/0.2.2) | `MIT` | não declarado |
| [tap 1.0.1](https://crates.io/crates/tap/1.0.1) | `MIT` | não declarado |
| [tempfile 3.27.0](https://crates.io/crates/tempfile/3.27.0) | `MIT OR Apache-2.0` | 1.63 |
| [termcolor 1.4.1](https://crates.io/crates/termcolor/1.4.1) | `Unlicense OR MIT` | não declarado |
| [thiserror 1.0.69](https://crates.io/crates/thiserror/1.0.69) | `MIT OR Apache-2.0` | 1.61 |
| [thiserror 2.0.21](https://crates.io/crates/thiserror/2.0.21) | `MIT OR Apache-2.0` | 1.77 |
| [thiserror-impl 1.0.69](https://crates.io/crates/thiserror-impl/1.0.69) | `MIT OR Apache-2.0` | 1.61 |
| [thiserror-impl 2.0.21](https://crates.io/crates/thiserror-impl/2.0.21) | `MIT OR Apache-2.0` | 1.77 |
| [tiff 0.11.3](https://crates.io/crates/tiff/0.11.3) | `MIT` | 1.85.0 |
| [time 0.3.55](https://crates.io/crates/time/0.3.55) | `MIT OR Apache-2.0` | 1.88.0 |
| [time-core 0.1.9](https://crates.io/crates/time-core/0.1.9) | `MIT OR Apache-2.0` | 1.88.0 |
| [time-macros 0.2.32](https://crates.io/crates/time-macros/0.2.32) | `MIT OR Apache-2.0` | 1.88.0 |
| [tiny-keccak 2.0.2](https://crates.io/crates/tiny-keccak/2.0.2) | `CC0-1.0` | não declarado |
| [tinystr 0.8.5](https://crates.io/crates/tinystr/0.8.5) | `Unicode-3.0` | 1.82 |
| [tinyvec 1.13.3](https://crates.io/crates/tinyvec/1.13.3) | `Zlib OR Apache-2.0 OR MIT` | não declarado |
| [tokio 1.52.3](https://crates.io/crates/tokio/1.52.3) | `MIT` | 1.71 |
| [tokio-macros 2.7.2](https://crates.io/crates/tokio-macros/2.7.2) | `MIT` | 1.71 |
| [tokio-rustls 0.26.6](https://crates.io/crates/tokio-rustls/0.26.6) | `MIT OR Apache-2.0` | 1.71 |
| [tokio-stream 0.1.19](https://crates.io/crates/tokio-stream/0.1.19) | `MIT` | 1.71 |
| [tokio-util 0.7.20](https://crates.io/crates/tokio-util/0.7.20) | `MIT` | 1.85 |
| [toml 1.1.8+spec-1.1.0](https://crates.io/crates/toml/1.1.8+spec-1.1.0) | `MIT OR Apache-2.0` | 1.85 |
| [toml_datetime 1.1.2+spec-1.1.0](https://crates.io/crates/toml_datetime/1.1.2+spec-1.1.0) | `MIT OR Apache-2.0` | 1.85 |
| [toml_edit 0.25.17+spec-1.1.0](https://crates.io/crates/toml_edit/0.25.17+spec-1.1.0) | `MIT OR Apache-2.0` | 1.85 |
| [toml_parser 1.1.5+spec-1.1.0](https://crates.io/crates/toml_parser/1.1.5+spec-1.1.0) | `MIT OR Apache-2.0` | 1.85 |
| [toml_writer 1.1.3+spec-1.1.0](https://crates.io/crates/toml_writer/1.1.3+spec-1.1.0) | `MIT OR Apache-2.0` | 1.85 |
| [tower 0.5.3](https://crates.io/crates/tower/0.5.3) | `MIT` | 1.64.0 |
| [tower-http 0.6.11](https://crates.io/crates/tower-http/0.6.11) | `MIT` | 1.64 |
| [tower-layer 0.3.3](https://crates.io/crates/tower-layer/0.3.3) | `MIT` | não declarado |
| [tower-service 0.3.3](https://crates.io/crates/tower-service/0.3.3) | `MIT` | não declarado |
| [tracing 0.1.44](https://crates.io/crates/tracing/0.1.44) | `MIT` | 1.65.0 |
| [tracing-attributes 0.1.31](https://crates.io/crates/tracing-attributes/0.1.31) | `MIT` | 1.65.0 |
| [tracing-core 0.1.36](https://crates.io/crates/tracing-core/0.1.36) | `MIT` | 1.65.0 |
| [tree-sitter 0.27.1](https://crates.io/crates/tree-sitter/0.27.1) | `MIT` | 1.90 |
| [tree-sitter-language 0.1.9](https://crates.io/crates/tree-sitter-language/0.1.9) | `MIT` | 1.90 |
| [tree-sitter-php 0.25.1](https://crates.io/crates/tree-sitter-php/0.25.1) | `MIT` | não declarado |
| [tree-sitter-python 0.25.0](https://crates.io/crates/tree-sitter-python/0.25.0) | `MIT` | não declarado |
| [tree-sitter-rust 0.24.2](https://crates.io/crates/tree-sitter-rust/0.24.2) | `MIT` | não declarado |
| [tree-sitter-typescript 0.23.2](https://crates.io/crates/tree-sitter-typescript/0.23.2) | `MIT` | não declarado |
| [tree_magic_mini 3.2.2](https://crates.io/crates/tree_magic_mini/3.2.2) | `MIT` | 1.85 |
| [try-lock 0.2.5](https://crates.io/crates/try-lock/0.2.5) | `MIT` | não declarado |
| [tungstenite 0.30.0](https://crates.io/crates/tungstenite/0.30.0) | `MIT OR Apache-2.0` | 1.85 |
| [type-map 0.5.1](https://crates.io/crates/type-map/0.5.1) | `MIT/Apache-2.0` | não declarado |
| [typed-builder 0.22.0](https://crates.io/crates/typed-builder/0.22.0) | `MIT OR Apache-2.0` | não declarado |
| [typed-builder-macro 0.22.0](https://crates.io/crates/typed-builder-macro/0.22.0) | `MIT OR Apache-2.0` | não declarado |
| [typed-path 0.12.4](https://crates.io/crates/typed-path/0.12.4) | `MIT OR Apache-2.0` | 1.65.0 |
| [typenum 1.20.1](https://crates.io/crates/typenum/1.20.1) | `MIT OR Apache-2.0` | 1.41.0 |
| [uds_windows 1.2.1](https://crates.io/crates/uds_windows/1.2.1) | `MIT` | 1.85 |
| [unicase 2.10.0](https://crates.io/crates/unicase/2.10.0) | `MIT OR Apache-2.0` | não declarado |
| [unicode-bidi 0.3.18](https://crates.io/crates/unicode-bidi/0.3.18) | `MIT OR Apache-2.0` | 1.47.0 |
| [unicode-general-category 1.1.0](https://crates.io/crates/unicode-general-category/1.1.0) | `Apache-2.0` | não declarado |
| [unicode-ident 1.0.27](https://crates.io/crates/unicode-ident/1.0.27) | `(MIT OR Apache-2.0) AND Unicode-3.0` | 1.71 |
| [unicode-normalization 0.1.26](https://crates.io/crates/unicode-normalization/0.1.26) | `(MIT OR Apache-2.0) AND Unicode-3.0` | 1.36 |
| [unicode-properties 0.1.4](https://crates.io/crates/unicode-properties/0.1.4) | `MIT/Apache-2.0` | não declarado |
| [unicode-segmentation 1.13.3](https://crates.io/crates/unicode-segmentation/1.13.3) | `MIT OR Apache-2.0` | 1.85.0 |
| [unicode-width 0.2.2](https://crates.io/crates/unicode-width/0.2.2) | `MIT OR Apache-2.0` | 1.66 |
| [unicode-xid 0.2.6](https://crates.io/crates/unicode-xid/0.2.6) | `MIT OR Apache-2.0` | 1.17 |
| [unicode_categories 0.1.1](https://crates.io/crates/unicode_categories/0.1.1) | `MIT OR Apache-2.0` | não declarado |
| [unsafe-libyaml 0.2.11](https://crates.io/crates/unsafe-libyaml/0.2.11) | `MIT` | 1.56 |
| [untrusted 0.9.0](https://crates.io/crates/untrusted/0.9.0) | `ISC` | não declarado |
| [url 2.5.8](https://crates.io/crates/url/2.5.8) | `MIT OR Apache-2.0` | 1.63 |
| [utf8_iter 1.0.4](https://crates.io/crates/utf8_iter/1.0.4) | `Apache-2.0 OR MIT` | não declarado |
| [uuid 1.28.0](https://crates.io/crates/uuid/1.28.0) | `Apache-2.0 OR MIT` | 1.89.0 |
| [vcpkg 0.2.15](https://crates.io/crates/vcpkg/0.2.15) | `MIT/Apache-2.0` | não declarado |
| [vello_common 0.2.0](https://crates.io/crates/vello_common/0.2.0) | `Apache-2.0 OR MIT` | 1.88 |
| [vello_cpu 0.2.0](https://crates.io/crates/vello_cpu/0.2.0) | `Apache-2.0 OR MIT` | 1.88 |
| [version_check 0.9.5](https://crates.io/crates/version_check/0.9.5) | `MIT/Apache-2.0` | não declarado |
| [vt100 0.18.1](https://crates.io/crates/vt100/0.18.1) | `MIT` | 1.85 |
| [vte 0.15.0](https://crates.io/crates/vte/0.15.0) | `Apache-2.0 OR MIT` | 1.62.1 |
| [walkdir 2.5.0](https://crates.io/crates/walkdir/2.5.0) | `Unlicense/MIT` | não declarado |
| [want 0.3.2](https://crates.io/crates/want/0.3.2) | `MIT` | não declarado |
| [wasi 0.11.1+wasi-snapshot-preview1](https://crates.io/crates/wasi/0.11.1+wasi-snapshot-preview1) | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | não declarado |
| [wasip2 1.0.4+wasi-0.2.12](https://crates.io/crates/wasip2/1.0.4+wasi-0.2.12) | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 1.87.0 |
| [wasite 0.1.0](https://crates.io/crates/wasite/0.1.0) | `Apache-2.0 OR BSL-1.0 OR MIT` | não declarado |
| [wasm-bindgen 0.2.129](https://crates.io/crates/wasm-bindgen/0.2.129) | `MIT OR Apache-2.0` | 1.81 |
| [wasm-bindgen-futures 0.4.79](https://crates.io/crates/wasm-bindgen-futures/0.4.79) | `MIT OR Apache-2.0` | 1.81 |
| [wasm-bindgen-macro 0.2.129](https://crates.io/crates/wasm-bindgen-macro/0.2.129) | `MIT OR Apache-2.0` | 1.81 |
| [wasm-bindgen-macro-support 0.2.129](https://crates.io/crates/wasm-bindgen-macro-support/0.2.129) | `MIT OR Apache-2.0` | 1.81 |
| [wasm-bindgen-shared 0.2.129](https://crates.io/crates/wasm-bindgen-shared/0.2.129) | `MIT OR Apache-2.0` | 1.81 |
| [wayland-backend 0.3.17](https://crates.io/crates/wayland-backend/0.3.17) | `MIT` | 1.71 |
| [wayland-client 0.31.15](https://crates.io/crates/wayland-client/0.31.15) | `MIT` | 1.71 |
| [wayland-csd-frame 0.3.0](https://crates.io/crates/wayland-csd-frame/0.3.0) | `MIT` | 1.65.0 |
| [wayland-cursor 0.31.14](https://crates.io/crates/wayland-cursor/0.31.14) | `MIT` | 1.71 |
| [wayland-protocols 0.32.13](https://crates.io/crates/wayland-protocols/0.32.13) | `MIT` | 1.86 |
| [wayland-protocols-experimental 20250721.0.1](https://crates.io/crates/wayland-protocols-experimental/20250721.0.1) | `MIT` | 1.65 |
| [wayland-protocols-misc 0.3.12](https://crates.io/crates/wayland-protocols-misc/0.3.12) | `MIT` | 1.71 |
| [wayland-protocols-plasma 0.3.12](https://crates.io/crates/wayland-protocols-plasma/0.3.12) | `MIT` | 1.71 |
| [wayland-protocols-wlr 0.3.12](https://crates.io/crates/wayland-protocols-wlr/0.3.12) | `MIT` | 1.71 |
| [wayland-scanner 0.31.11](https://crates.io/crates/wayland-scanner/0.31.11) | `MIT` | 1.71 |
| [wayland-sys 0.31.11](https://crates.io/crates/wayland-sys/0.31.11) | `MIT` | 1.71 |
| [web-sys 0.3.106](https://crates.io/crates/web-sys/0.3.106) | `MIT OR Apache-2.0` | 1.81 |
| [web-time 1.1.0](https://crates.io/crates/web-time/1.1.0) | `MIT OR Apache-2.0` | 1.60 |
| [webbrowser 1.2.6](https://crates.io/crates/webbrowser/1.2.6) | `MIT OR Apache-2.0` | 1.85 |
| [webpki-roots 0.26.11](https://crates.io/crates/webpki-roots/0.26.11) | `CDLA-Permissive-2.0` | não declarado |
| [webpki-roots 1.0.11](https://crates.io/crates/webpki-roots/1.0.11) | `CDLA-Permissive-2.0` | 1.71 |
| [weezl 0.1.12](https://crates.io/crates/weezl/0.1.12) | `MIT OR Apache-2.0` | não declarado |
| [wgpu 30.0.1](https://crates.io/crates/wgpu/30.0.1) | `MIT OR Apache-2.0` | 1.87.0 |
| [wgpu-core 30.0.1](https://crates.io/crates/wgpu-core/30.0.1) | `MIT OR Apache-2.0` | 1.87 |
| [wgpu-core-deps-apple 30.0.1](https://crates.io/crates/wgpu-core-deps-apple/30.0.1) | `MIT OR Apache-2.0` | 1.76 |
| [wgpu-core-deps-emscripten 30.0.1](https://crates.io/crates/wgpu-core-deps-emscripten/30.0.1) | `MIT OR Apache-2.0` | 1.76 |
| [wgpu-core-deps-wasm 30.0.1](https://crates.io/crates/wgpu-core-deps-wasm/30.0.1) | `MIT OR Apache-2.0` | 1.76 |
| [wgpu-core-deps-windows-linux-android 30.0.1](https://crates.io/crates/wgpu-core-deps-windows-linux-android/30.0.1) | `MIT OR Apache-2.0` | 1.76 |
| [wgpu-hal 30.0.1](https://crates.io/crates/wgpu-hal/30.0.1) | `MIT OR Apache-2.0` | 1.87 |
| [wgpu-naga-bridge 30.0.1](https://crates.io/crates/wgpu-naga-bridge/30.0.1) | `MIT OR Apache-2.0` | 1.87 |
| [wgpu-types 30.0.1](https://crates.io/crates/wgpu-types/30.0.1) | `MIT OR Apache-2.0` | 1.87 |
| [whoami 1.6.1](https://crates.io/crates/whoami/1.6.1) | `Apache-2.0 OR BSL-1.0 OR MIT` | 1.40 |
| [widestring 1.2.1](https://crates.io/crates/widestring/1.2.1) | `MIT OR Apache-2.0` | 1.58 |
| [winapi 0.3.9](https://crates.io/crates/winapi/0.3.9) | `MIT/Apache-2.0` | não declarado |
| [winapi-i686-pc-windows-gnu 0.4.0](https://crates.io/crates/winapi-i686-pc-windows-gnu/0.4.0) | `MIT/Apache-2.0` | não declarado |
| [winapi-util 0.1.11](https://crates.io/crates/winapi-util/0.1.11) | `Unlicense OR MIT` | não declarado |
| [winapi-x86_64-pc-windows-gnu 0.4.0](https://crates.io/crates/winapi-x86_64-pc-windows-gnu/0.4.0) | `MIT/Apache-2.0` | não declarado |
| [windows 0.62.2](https://crates.io/crates/windows/0.62.2) | `MIT OR Apache-2.0` | 1.82 |
| [windows-collections 0.3.2](https://crates.io/crates/windows-collections/0.3.2) | `MIT OR Apache-2.0` | 1.82 |
| [windows-core 0.62.2](https://crates.io/crates/windows-core/0.62.2) | `MIT OR Apache-2.0` | 1.82 |
| [windows-future 0.3.2](https://crates.io/crates/windows-future/0.3.2) | `MIT OR Apache-2.0` | 1.82 |
| [windows-implement 0.60.2](https://crates.io/crates/windows-implement/0.60.2) | `MIT OR Apache-2.0` | 1.74 |
| [windows-interface 0.59.3](https://crates.io/crates/windows-interface/0.59.3) | `MIT OR Apache-2.0` | 1.74 |
| [windows-link 0.2.1](https://crates.io/crates/windows-link/0.2.1) | `MIT OR Apache-2.0` | 1.71 |
| [windows-numerics 0.3.1](https://crates.io/crates/windows-numerics/0.3.1) | `MIT OR Apache-2.0` | 1.82 |
| [windows-registry 0.6.1](https://crates.io/crates/windows-registry/0.6.1) | `MIT OR Apache-2.0` | 1.82 |
| [windows-result 0.4.1](https://crates.io/crates/windows-result/0.4.1) | `MIT OR Apache-2.0` | 1.82 |
| [windows-strings 0.5.1](https://crates.io/crates/windows-strings/0.5.1) | `MIT OR Apache-2.0` | 1.82 |
| [windows-sys 0.48.0](https://crates.io/crates/windows-sys/0.48.0) | `MIT OR Apache-2.0` | 1.48 |
| [windows-sys 0.52.0](https://crates.io/crates/windows-sys/0.52.0) | `MIT OR Apache-2.0` | 1.56 |
| [windows-sys 0.59.0](https://crates.io/crates/windows-sys/0.59.0) | `MIT OR Apache-2.0` | 1.60 |
| [windows-sys 0.60.2](https://crates.io/crates/windows-sys/0.60.2) | `MIT OR Apache-2.0` | 1.60 |
| [windows-sys 0.61.2](https://crates.io/crates/windows-sys/0.61.2) | `MIT OR Apache-2.0` | 1.71 |
| [windows-targets 0.48.5](https://crates.io/crates/windows-targets/0.48.5) | `MIT OR Apache-2.0` | não declarado |
| [windows-targets 0.52.6](https://crates.io/crates/windows-targets/0.52.6) | `MIT OR Apache-2.0` | 1.56 |
| [windows-targets 0.53.5](https://crates.io/crates/windows-targets/0.53.5) | `MIT OR Apache-2.0` | 1.60 |
| [windows-threading 0.2.1](https://crates.io/crates/windows-threading/0.2.1) | `MIT OR Apache-2.0` | 1.82 |
| [windows_aarch64_gnullvm 0.48.5](https://crates.io/crates/windows_aarch64_gnullvm/0.48.5) | `MIT OR Apache-2.0` | não declarado |
| [windows_aarch64_gnullvm 0.52.6](https://crates.io/crates/windows_aarch64_gnullvm/0.52.6) | `MIT OR Apache-2.0` | 1.56 |
| [windows_aarch64_gnullvm 0.53.1](https://crates.io/crates/windows_aarch64_gnullvm/0.53.1) | `MIT OR Apache-2.0` | 1.60 |
| [windows_aarch64_msvc 0.48.5](https://crates.io/crates/windows_aarch64_msvc/0.48.5) | `MIT OR Apache-2.0` | não declarado |
| [windows_aarch64_msvc 0.52.6](https://crates.io/crates/windows_aarch64_msvc/0.52.6) | `MIT OR Apache-2.0` | 1.56 |
| [windows_aarch64_msvc 0.53.1](https://crates.io/crates/windows_aarch64_msvc/0.53.1) | `MIT OR Apache-2.0` | 1.60 |
| [windows_i686_gnu 0.48.5](https://crates.io/crates/windows_i686_gnu/0.48.5) | `MIT OR Apache-2.0` | não declarado |
| [windows_i686_gnu 0.52.6](https://crates.io/crates/windows_i686_gnu/0.52.6) | `MIT OR Apache-2.0` | 1.56 |
| [windows_i686_gnu 0.53.1](https://crates.io/crates/windows_i686_gnu/0.53.1) | `MIT OR Apache-2.0` | 1.60 |
| [windows_i686_gnullvm 0.52.6](https://crates.io/crates/windows_i686_gnullvm/0.52.6) | `MIT OR Apache-2.0` | 1.56 |
| [windows_i686_gnullvm 0.53.1](https://crates.io/crates/windows_i686_gnullvm/0.53.1) | `MIT OR Apache-2.0` | 1.60 |
| [windows_i686_msvc 0.48.5](https://crates.io/crates/windows_i686_msvc/0.48.5) | `MIT OR Apache-2.0` | não declarado |
| [windows_i686_msvc 0.52.6](https://crates.io/crates/windows_i686_msvc/0.52.6) | `MIT OR Apache-2.0` | 1.56 |
| [windows_i686_msvc 0.53.1](https://crates.io/crates/windows_i686_msvc/0.53.1) | `MIT OR Apache-2.0` | 1.60 |
| [windows_x86_64_gnu 0.48.5](https://crates.io/crates/windows_x86_64_gnu/0.48.5) | `MIT OR Apache-2.0` | não declarado |
| [windows_x86_64_gnu 0.52.6](https://crates.io/crates/windows_x86_64_gnu/0.52.6) | `MIT OR Apache-2.0` | 1.56 |
| [windows_x86_64_gnu 0.53.1](https://crates.io/crates/windows_x86_64_gnu/0.53.1) | `MIT OR Apache-2.0` | 1.60 |
| [windows_x86_64_gnullvm 0.48.5](https://crates.io/crates/windows_x86_64_gnullvm/0.48.5) | `MIT OR Apache-2.0` | não declarado |
| [windows_x86_64_gnullvm 0.52.6](https://crates.io/crates/windows_x86_64_gnullvm/0.52.6) | `MIT OR Apache-2.0` | 1.56 |
| [windows_x86_64_gnullvm 0.53.1](https://crates.io/crates/windows_x86_64_gnullvm/0.53.1) | `MIT OR Apache-2.0` | 1.60 |
| [windows_x86_64_msvc 0.48.5](https://crates.io/crates/windows_x86_64_msvc/0.48.5) | `MIT OR Apache-2.0` | não declarado |
| [windows_x86_64_msvc 0.52.6](https://crates.io/crates/windows_x86_64_msvc/0.52.6) | `MIT OR Apache-2.0` | 1.56 |
| [windows_x86_64_msvc 0.53.1](https://crates.io/crates/windows_x86_64_msvc/0.53.1) | `MIT OR Apache-2.0` | 1.60 |
| [winit 0.30.13](https://crates.io/crates/winit/0.30.13) | `Apache-2.0` | 1.70.0 |
| [winnow 0.7.15](https://crates.io/crates/winnow/0.7.15) | `MIT` | 1.65.0 |
| [winnow 1.0.4](https://crates.io/crates/winnow/1.0.4) | `MIT` | 1.65.0 |
| [winreg 0.10.1](https://crates.io/crates/winreg/0.10.1) | `MIT` | não declarado |
| [wit-bindgen 0.57.1](https://crates.io/crates/wit-bindgen/0.57.1) | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 1.85.0 |
| [wl-clipboard-rs 0.9.4](https://crates.io/crates/wl-clipboard-rs/0.9.4) | `MIT OR Apache-2.0` | não declarado |
| [writeable 0.6.4](https://crates.io/crates/writeable/0.6.4) | `Unicode-3.0` | 1.82 |
| [wyz 0.5.1](https://crates.io/crates/wyz/0.5.1) | `MIT` | não declarado |
| [x11-dl 2.21.0](https://crates.io/crates/x11-dl/2.21.0) | `MIT` | não declarado |
| [x11rb 0.13.2](https://crates.io/crates/x11rb/0.13.2) | `MIT OR Apache-2.0` | 1.64 |
| [x11rb-protocol 0.13.2](https://crates.io/crates/x11rb-protocol/0.13.2) | `MIT OR Apache-2.0` | 1.64 |
| [xcursor 0.3.11](https://crates.io/crates/xcursor/0.3.11) | `MIT` | não declarado |
| [xdg-home 1.3.0](https://crates.io/crates/xdg-home/1.3.0) | `MIT` | 1.60 |
| [xkbcommon-dl 0.4.2](https://crates.io/crates/xkbcommon-dl/0.4.2) | `MIT` | 1.64 |
| [xkeysym 0.2.1](https://crates.io/crates/xkeysym/0.2.1) | `MIT OR Apache-2.0 OR Zlib` | 1.58.1 |
| [xml-rs 0.8.29](https://crates.io/crates/xml-rs/0.8.29) | `MIT` | 1.70 |
| [yoke 0.8.4](https://crates.io/crates/yoke/0.8.4) | `Unicode-3.0` | 1.82 |
| [yoke-derive 0.8.4](https://crates.io/crates/yoke-derive/0.8.4) | `Unicode-3.0` | 1.82 |
| [zbus 4.4.0](https://crates.io/crates/zbus/4.4.0) | `MIT` | 1.75 |
| [zbus 5.19.0](https://crates.io/crates/zbus/5.19.0) | `MIT` | 1.87 |
| [zbus-lockstep 0.5.2](https://crates.io/crates/zbus-lockstep/0.5.2) | `MIT` | não declarado |
| [zbus-lockstep-macros 0.5.2](https://crates.io/crates/zbus-lockstep-macros/0.5.2) | `MIT` | não declarado |
| [zbus_macros 4.4.0](https://crates.io/crates/zbus_macros/4.4.0) | `MIT` | 1.75 |
| [zbus_macros 5.19.0](https://crates.io/crates/zbus_macros/5.19.0) | `MIT` | 1.87 |
| [zbus_names 3.0.0](https://crates.io/crates/zbus_names/3.0.0) | `MIT` | 1.75 |
| [zbus_names 4.3.5](https://crates.io/crates/zbus_names/4.3.5) | `MIT` | 1.87 |
| [zbus_xml 5.2.1](https://crates.io/crates/zbus_xml/5.2.1) | `MIT` | 1.87 |
| [zcheapstr 1.1.0](https://crates.io/crates/zcheapstr/1.1.0) | `MIT` | 1.87 |
| [zerocopy 0.8.63](https://crates.io/crates/zerocopy/0.8.63) | `BSD-2-Clause OR Apache-2.0 OR MIT` | 1.56.0 |
| [zerocopy-derive 0.8.63](https://crates.io/crates/zerocopy-derive/0.8.63) | `BSD-2-Clause OR Apache-2.0 OR MIT` | não declarado |
| [zerofrom 0.1.8](https://crates.io/crates/zerofrom/0.1.8) | `Unicode-3.0` | 1.71.1 |
| [zerofrom-derive 0.1.8](https://crates.io/crates/zerofrom-derive/0.1.8) | `Unicode-3.0` | 1.71.1 |
| [zeroize 1.9.1](https://crates.io/crates/zeroize/1.9.1) | `Apache-2.0 OR MIT` | 1.85 |
| [zeroize_derive 1.5.0](https://crates.io/crates/zeroize_derive/1.5.0) | `Apache-2.0 OR MIT` | 1.85 |
| [zerotrie 0.2.5](https://crates.io/crates/zerotrie/0.2.5) | `Unicode-3.0` | 1.82 |
| [zerovec 0.11.9](https://crates.io/crates/zerovec/0.11.9) | `Unicode-3.0` | 1.83 |
| [zerovec-derive 0.11.7](https://crates.io/crates/zerovec-derive/0.11.7) | `Unicode-3.0` | 1.83 |
| [zip 9.0.5](https://crates.io/crates/zip/9.0.5) | `MIT` | 1.88 |
| [zlib-rs 0.6.8](https://crates.io/crates/zlib-rs/0.6.8) | `Zlib` | 1.75 |
| [zmij 1.0.23](https://crates.io/crates/zmij/1.0.23) | `MIT` | 1.71 |
| [zune-core 0.5.3](https://crates.io/crates/zune-core/0.5.3) | `MIT OR Apache-2.0 OR Zlib` | 1.75.0 |
| [zune-jpeg 0.5.15](https://crates.io/crates/zune-jpeg/0.5.15) | `MIT OR Apache-2.0 OR Zlib` | 1.75.0 |
| [zvariant 4.2.0](https://crates.io/crates/zvariant/4.2.0) | `MIT` | 1.75 |
| [zvariant 5.15.1](https://crates.io/crates/zvariant/5.15.1) | `MIT` | 1.87 |
| [zvariant_derive 4.2.0](https://crates.io/crates/zvariant_derive/4.2.0) | `MIT` | 1.75 |
| [zvariant_derive 5.15.1](https://crates.io/crates/zvariant_derive/5.15.1) | `MIT` | 1.87 |
| [zvariant_utils 2.1.0](https://crates.io/crates/zvariant_utils/2.1.0) | `MIT` | 1.75 |
| [zvariant_utils 4.2.0](https://crates.io/crates/zvariant_utils/4.2.0) | `MIT` | 1.87 |

<!-- END CARGO METADATA INVENTORY -->
