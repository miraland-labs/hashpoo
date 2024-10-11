# Poolore - Easy and Simplified Public Pool for ORE Mining

**The goal of this project is to bring more consumer miners on board through easy and simple ORE mining operations, in line with the ORE design principle: anyone can mine.**

**This is Poolore client, originated from [ore-private-pool-cli](https://github.com/miraland-labs/ore-private-pool-cli.git) which derived from [ore-hq-client](https://github.com/Kriptikz/ore-hq-client.git).**

## Key Differentiators of Poolore

**Fully embrace and leverage ORE program**

**No sign up fee, no sign up action**

**No delegating operation, no manual staking**

**Start mining when you connect. Stop mining when you disconnect.**

**Claim anytime at your own discretion (with reward balance >= 0.005 ORE).**

**Suitable for both casual and professional miners**

Poolorer(Poolore community member) are welcome to join discord server:

-   [Mirapoo Discord](https://discord.gg/YjQhWqxp7H)

## Install

Poolore client(poolore-cli) installation steps:

```sh
cargo install poolore-cli
```

Source code located at Github: [github](https://github.com/miraland-labs/poolore)

### Dependencies

If you run into issues during installation, please install the following dependencies for your operating system and try again:

#### Linux

```
sudo apt-get install openssl pkg-config libssl-dev gcc
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

## Run

To run poolore client, execute:

```sh
poolorec [OPTIONS]
```

or, if you can reference command scripts example file under [poolore github](https://github.com/miraland-labs/poolore)

```sh
scripts/start-poolore-cli.sh
```

## Help

You can use the `-h` flag on any command to pull up a help menu with documentation:

```sh
poolorec -h

Usage: poolorec [OPTIONS] <COMMAND>

Commands:
  mine       Connect to pool and start mining. (Default Implementation)
  protomine  Connect to pool and start mining. (Protomine Implementation)
  help       Print this message or the help of the given subcommand(s)

Options:
      --url <SERVER_URL>        Host name and port of your private pool server to connect to, it can also be your LAN ip address:port like: 172.xxx.xx.xxx:3000, 192.xxx.xx.xxx:3000 [default: orepool.miraland.io:3000]
      --keypair <KEYPAIR_PATH>  Filepath to keypair to use [default: ~/.config/solana/id.json]
  -u, --use-http                Use unsecure http connection instead of https.
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
