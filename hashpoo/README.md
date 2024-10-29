# Hashpoo - Hashpower pool only, for PoW believers. Simple and easy public pool dedicated to ORE mining, pure PoW, no staking no boosting.

**The goal of this project is to bring more consumer miners on board through easy and simple ORE mining operations, in line with the ORE design principle: anyone can mine.**

**This is Hashpoo server.**

## Key Differentiators of Hashpoo

**No sign up fee, no sign up action**

**Hashpoo does NOT cap or limit your hashpower in any way.**

**No delegating, no staking, no boosting, pure PoW**

**Start mining when you connect. Stop mining when you disconnect.**

**Claim anytime at your own discretion (with reward balance >= 0.005 ORE).**

**Suitable for both casual and professional miners**

Hashpoo community members(Hashpunkers) are welcome to join discord server:

-   [Mirapoo Discord](https://discord.gg/YjQhWqxp7H)

## Install

Hashpoo server(Hashpoo) installation steps:

To install the Hashpoo pool server, 2 approaches are recommended:

**Approach One**: install from crates.io directly, use [cargo](https://doc.rust-lang.org/cargo/getting-started/installation.html):

```sh
cargo install hashpoo
```

**Approach Two**: download source code from Github at: [github](https://github.com/miraland-labs/hashpoo):

```sh
https://github.com/miraland-labs/hashpoo
```

and then compile locally

`cargo build --release`

### Dependencies

If you run into issues during installation, please install the following dependencies for your operating system and try again:

#### Linux

```
sudo apt-get update
sudo apt-get install openssl pkg-config libssl-dev gcc
```

Notes: you may need to install dependencies to enable sound notification on ubuntu OS with:

```
sudo apt-get install libasound2-dev
sudo apt-get install libudev-dev
```

#### MacOS (using [Homebrew](https://brew.sh/))

```
brew install openssl pkg-config

# If you encounter issues with OpenSSL, you might need to set the following environment variables:
export PATH="/usr/local/opt/openssl/bin:$PATH"
export LDFLAGS="-L/usr/local/opt/openssl/lib"
export CPPFLAGS="-I/usr/local/opt/openssl/include"
```

#### Windows (using [Chocolatey](https://chocolatey.org/))

```
choco install openssl pkgconfiglite
```

#### rust (if not installed yet)

Open a terminal window on Mac / Linux / BSD / Windows PowerShell:

```
curl https://sh.rustup.rs -sSf | sh
```

or

```
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Build

To build the codebase from scratch, checkout the repo and use cargo to build:

```sh
cargo build --release
```

## Env Settings

Navigate to server sub directory, copy `.env.example` file and rename to `.env`, set with your own settings.

## Run

To run hashpoo server, execute:

```sh
hps [OPTIONS]
```

or, if you build from source code downloaded from github, enter into server sub directory,
duplicate `scripts.example` directory and rename to `scripts`, modify settings in `hps.sh`, execute:

```
./hps.sh
```

or

```
RUST_LOG=info ./hps.sh
```

There are 2 types of daily log files in the `logs' subdirectory. One type is server_log with the filename pattern hashpoo.log.yyyy-mm-dd, the other type is contribution_log with the filename pattern contributions.log.yyyy-mm-dd. By default, the server log with filtered content is redirected to standard output or the terminal.

## Help

You can use the `-h` flag on any command to pull up a help menu with documentation:

```sh
hps -h

Usage: hps [OPTIONS]

Options:
-b, --buffer-time <BUFFER_SECONDS>
        The number seconds before the deadline to stop mining and start submitting. [default: 5]
-r, --risk-time <RISK_SECONDS>
        Set extra hash time in seconds for miners to stop mining and start submitting, risking a penalty. [default: 0]
--priority-fee <FEE_MICROLAMPORTS>
        Price to pay for compute units when dynamic fee flag is off, or dynamic fee is unavailable. [default: 100]
--priority-fee-cap <FEE_CAP_MICROLAMPORTS>
        Max price to pay for compute units when dynamic fees are enabled. [default: 100000]
--dynamic-fee
        Enable dynamic priority fees
--dynamic-fee-url <DYNAMIC_FEE_URL>
        RPC URL to use for dynamic fee estimation.
-e, --expected-min-difficulty <EXPECTED_MIN_DIFFICULTY>
        The expected min difficulty to submit from pool client. Reserved for potential qualification process unimplemented yet. [default: 8]
-e, --extra-fee-difficulty <EXTRA_FEE_DIFFICULTY>
        The min difficulty that the pool server miner thinks deserves to pay more priority fee to land tx quickly. [default: 29]
-e, --extra-fee-percent <EXTRA_FEE_PERCENT>
        The extra percentage that the pool server miner feels deserves to pay more of the priority fee. As a percentage, a multiple of 50 is recommended(example: 50, means pay extra 50% of the specified priority fee), and the final priority fee cannot exceed the priority fee cap. [default: 0]
-s, --slack-difficulty <SLACK_DIFFICULTY>
        The min difficulty that will notify slack channel(if configured) upon transaction success. [default: 25]
  -m, --messaging-diff <MESSAGING_DIFF>
        The min difficulty that will notify messaging channels(if configured) upon transaction success. [default: 25]
      --send-tpu-mine-tx
        Send and confirm transactions using tpu client.
--no-sound-notification
        Sound notification on by default
```

## Support us | Donate at your discretion

We greatly appreciate any donation to help support projects development at Miraland Labs. Miraland is dedicated to freedom and individual sovereignty and we are doing our best to make it a reality.
Certainly, if you find this project helpful and would like to support its development, you can buy me/us a coffee!
Your support is greatly appreciated. It motivates me/us to keep improving this project.

**Bitcoin(BTC)**
`bc1plh7wnl0v0xfemmk395tvsu73jtt0s8l28lhhznafzrj5jwu4dy9qx2rpda`

![Donate BTC to Miraland Development](../donations/donate-btc-qr-code.png)

**Solana(SOL)**
`9h9TXFtSsDAiL5kpCRZuKUxPE4Nv3W56fcSyUC3zmQip`

![Donate SOL to Miraland Development](../donations/donate-sol-qr-code.png)

Thank you for your support!
