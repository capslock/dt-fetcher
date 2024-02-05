# dt-fetcher

[![CI](https://github.com/capslock/dt-fetcher/actions/workflows/ci.yml/badge.svg)](https://github.com/capslock/dt-fetcher/actions?query=workflow%3ACI+event%3Apush)

## Install

```console
cargo install --git https://github.com/capslock/dt-fetcher
```

## Usage

```console
> dt-fetcher -h
Usage: dt-fetcher [OPTIONS]

Options:
      --auth <AUTH>                Path to auth json file
      --listen-addr <LISTEN_ADDR>  Host and port to listen on [default: 0.0.0.0:3000]
      --log-to-systemd             Output logs directly to systemd
  -h, --help                       Print help
```

## API

### Single Account

These endpoints are applicable when there is only a single account provided to
`dt-fetcher`.

#### `GET /store`

Get store contents for the specified character and currency type.

##### Parameters

| parameter      | description          |
| -------------- | -------------------- |
| `characterId`  | `uuid` of character  |
| `currencyType` | `credits` or `marks` |

#### `GET /summary`

Get account summary.

#### `GET /master_data`

Get master data info.

### Multi-Account

These endpoints are applicable when one or more accounts are provided to
`dt-fetcher`.

#### `GET /store/:id`

Get store contents for the specified character and currency type.

##### Parameters

`:id`: UUID of the account.

| Parameter      | Description          |
| -------------- | -------------------- |
| `characterId`  | `uuid` of character  |
| `currencyType` | `credits` or `marks` |

#### `GET /summary/:id`

Get account summary.

##### Parameters

`:id`: UUID of the account.

#### `GET /master_data/:id`

Get master data info.

##### Parameters

`:id`: UUID of the account.

### Auth

This endpoint is always available and can be used to provide accounts to `dt-fetcher`.

#### `PUT /auth`

Put a JSON auth object to have `dt-fetcher` manage the lifecycle and enable the
other endpoints for the associated account.
