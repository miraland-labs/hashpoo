# Hashpoo: Hashing, once started, cannot be stopped at all. (Experimental)

## Hashpoo - for real hashpower only, specifically tailored for PoW believers and/or hashpower paranoids. It's a public oriented, simple and easy to join pool dedicated to ORE mining. It features pure PoW, no staking, no boosting, absolutely compelled by the real and physical hashpower of the entire community. Have fun.

**The goal of this project is to bring more consumer miners on board through easy and simple ORE mining operations, in line with the ORE design principle: anyone can mine.**

**This is Hashpoo client.**

## Key Differentiators of Hashpoo

**No sign up fee, no sign up action.**

**Hashpoo does NOT cap or limit your hashpower in any way.**

**No delegating, no staking, no boosting, pure PoW, only hashpower.**

**Start mining when you connect. Stop mining when you disconnect.**

**Claim anytime at your own discretion (with reward balance >= 0.005 ORE).**

**Suitable for both casual and professional miners.**

Hashpoo community members(Hashpunkers) are welcome to join discord server:

-   [Mirapoo Discord](https://discord.gg/YjQhWqxp7H)

## Install

Hashpoo client(hashpoo-cli) can be installed from crates.io directly, use [cargo](https://doc.rust-lang.org/cargo/getting-started/installation.html):

```sh
cargo install hashpoo-cli
```

Source code located at Github: [github](https://github.com/miraland-labs/hashpoo)

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

To run hashpoo client, execute:

```sh
hpc [OPTIONS]
```

or, if you reference command scripts example file under [hashpoo github](https://github.com/miraland-labs/hashpoo)

```sh
scripts/hpc.sh
```

## Help

You can use the `-h` flag on any command to pull up a help menu with documentation:

```sh
hpc -h

Usage: hpc [OPTIONS] [COMMAND]

Commands:
  mine       Connect to pool and start mining. (Default Impl.)
  turbomine  Connect to pool and start mining. (Turbomine Impl.)
  claim      Claim rewards.
  balance    Display current ORE token balance.
  keygen     Generate a new Solana keypair for mining.
  earnings   Displays locally tracked earnings.
  help       Print this message or the help of the given subcommand(s)

Options:
      --url <SERVER_URL>        URL of the hashpoo server to connect to [default: ore.hashspace.me]
      --keypair <KEYPAIR_PATH>  Filepath to keypair to use [default: ~/.config/solana/id.json]
  -u, --use-http                Use unsecure http connection instead of https.
  -v, --vim                     Use vim mode for menu navigation.
  -h, --help                    Print help
  -V, --version                 Print version
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
