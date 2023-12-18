# dt-fetcher

## Install

```console
cargo install --git https://github.com/capslock/dt-fetcher
```

## Usage

```console
> dt-fetcher -h
Usage: dt-fetcher.exe [OPTIONS]

Options:
      --auth <AUTH>                Path to auth json file [default: auth.json]
      --listen-addr <LISTEN_ADDR>  Host and port to listen on [default: 0.0.0.0:3000]
  -h, --help                       Print help
```

## API

### `GET /store`

Get store contents for the specified character and currency type.

#### Parameters

| Parameter      | Description          |
| -------------- | -------------------- |
| `characterId`  | `uuid` of character  |
| `currencyType` | `credits` or `marks` |

### `GET /summary`

Get account summary.

### `GET /master_data`

Get master data info.
