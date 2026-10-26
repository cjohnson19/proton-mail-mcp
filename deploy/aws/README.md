# Deploying proton-mail-mcp to AWS

A CDK app that runs the MCP server as an always-on remote endpoint on a
single EC2 instance, so Claude (web, mobile, desktop) or ChatGPT can reach
your Proton mailbox without your laptop being on.

```
Internet :443
   │
   ▼
Caddy (host)            TLS via Let's Encrypt, reverse proxy to :8080
   │
   ▼
proton-mail-mcp         Docker container, streamable-HTTP + OAuth 2.1 on /mcp
   │
   ▼
Proton Mail Bridge      Docker container, IMAP on the internal docker network
   │
   ▼
Proton
```

The server enforces OAuth itself (passphrase-gated authorize page, PKCE,
short-lived JWTs). Nothing but ports 80/443 is exposed, and there is no SSH:
you reach the box through SSM Session Manager.

Secrets never live in this folder. They live in SSM Parameter Store and the
instance reads them at boot and every 30 minutes afterwards.

## What you need

- An AWS account with the [CDK bootstrapped](https://docs.aws.amazon.com/cdk/v2/guide/bootstrapping.html)
  in your target region (`npx cdk bootstrap aws://ACCOUNT/REGION`).
- A **public Route 53 hosted zone** for a domain you own. The stack creates
  one A record inside it (e.g. `mail-mcp.example.com`).
- The **default VPC** in the region (every account has one unless it was
  deleted).
- AWS CLI v2 logged in, Node.js 20+, and Docker running locally (the app image
  is built on your machine and pushed to ECR).
- A paid Proton plan. Proton Mail Bridge requires one.

Running cost is roughly USD 10 to 15 per month: a `t3a.micro`, a 16 GB gp3
volume, and one Elastic IP.

## 1. Configure

```bash
cd deploy/aws
npm install
cp config.example.json config.json
```

Edit `config.json` (gitignored):

| Field | Required | Meaning |
|-------|----------|---------|
| `account` | yes | 12-digit AWS account id |
| `region` | yes | Region to deploy into |
| `domainName` | yes | Public hostname for the endpoint, e.g. `mail-mcp.example.com` |
| `hostedZoneName` | yes | The Route 53 zone that contains `domainName`, e.g. `example.com` |
| `instanceType` | no | Default `t3a.micro`. A larger instance speeds up Bridge's initial mailbox sync; you can change it later without replacing the instance |
| `amiId` | no | Debian 12 AMI to pin. Leave `null` for the first deploy, then pin it (see below) |
| `bridgeImage` | no | Community Bridge image. Default is pinned to a known-good tag |
| `ssmPrefix` | no | Parameter Store prefix, default `/proton-mail-mcp` |
| `stackName` | no | CloudFormation stack name, default `MailMcpStack` |
| `oauthRedirectAllowlist` | no | Extra OAuth redirect URIs. Claude, ChatGPT and loopback are built in |

## 2. Create the secrets

The instance expects these parameters to exist before it boots. The Bridge
username and password are not known yet, so start with placeholders:

```bash
PREFIX=/proton-mail-mcp
aws ssm put-parameter --name $PREFIX/oauth-passphrase --type SecureString --value '<a passphrase of 12+ characters you will type once per client>'
aws ssm put-parameter --name $PREFIX/oauth-signing-key --type SecureString --value "$(openssl rand -hex 32)"
aws ssm put-parameter --name $PREFIX/username --type SecureString --value placeholder
aws ssm put-parameter --name $PREFIX/password --type SecureString --value placeholder
```

A fifth parameter, `image-uri`, is written by `cdk deploy`. Never edit it by
hand.

## 3. Deploy

```bash
npx cdk diff      # optional preview
npx cdk deploy
```

The first deploy builds the Rust image (several minutes), creates the
instance, and prints four outputs: `ConnectorUrl`, `PublicIp`, `InstanceId`,
and `AmiId`.

**Pin the AMI now.** Copy the `AmiId` output into `config.json`. Without the
pin, Debian's "latest" image moves over time and the next deploy would
replace your instance, which wipes the Bridge login you are about to do.

Wait a couple of minutes for cloud-init to finish, then check that Caddy got
its certificate:

```bash
curl -s https://mail-mcp.example.com/healthz   # -> ok
```

## 4. Log Bridge into your Proton account

This is interactive and happens once (and again whenever the instance is
replaced). Bridge keeps its state in `/opt/bridge-data` on the instance.

```bash
aws ssm start-session --target <InstanceId>
sudo docker exec -it protonmail-bridge /bin/bash
# inside the container:
pkill bridge            # only one bridge process may run at a time
/usr/bin/bridge --cli
#   >>> login           # Proton email, password, 2FA; wait for the sync to finish
#   >>> info            # prints the Bridge IMAP username and generated password
#   >>> exit
exit
sudo systemctl restart protonmail-bridge
```

Store the values from `info` and tell the instance to pick them up:

```bash
aws ssm put-parameter --overwrite --name $PREFIX/username --type SecureString --value '<bridge username>'
aws ssm put-parameter --overwrite --name $PREFIX/password --type SecureString --value '<bridge password>'
aws ssm send-command --instance-ids <InstanceId> --document-name AWS-RunShellScript \
  --parameters 'commands=/usr/local/bin/mcp-refresh-config'
```

## 5. Smoke test and connect

```bash
curl -s https://mail-mcp.example.com/healthz                                        # -> ok
curl -s -o /dev/null -w '%{http_code}\n' https://mail-mcp.example.com/mcp           # -> 401
curl -s https://mail-mcp.example.com/.well-known/oauth-protected-resource/mcp | jq  # -> resource metadata
```

**Claude:** Settings, Connectors, Add custom connector. Paste the
`ConnectorUrl` and leave the OAuth fields blank. Claude registers itself,
opens the authorize page, and you type the passphrase once.

**ChatGPT:** see the "ChatGPT" section of the repository README for the
connector setup. The server already allows ChatGPT's callback URL.

## Day-to-day operations

### Shipping a new server version

```bash
git pull
cd deploy/aws && npx cdk deploy
```

Only the image and the `image-uri` parameter change. The instance is not
touched. A State Manager association runs `mcp-refresh-config` every 30
minutes and restarts the server when it sees a new image. To roll out
immediately, run the `send-command` from step 4.

### Changing the infrastructure

Anything that alters the instance's user data (the refresh script, the
systemd units, packages) or the AMI **replaces the instance**. That loses the
Bridge login, so plan to redo step 4 afterwards. `npx cdk diff` tells you in
advance whether the instance will be replaced.

It is possible to carry `/opt/bridge-data` over to the new instance, but
Bridge must stay stopped from the moment you tar it until it is restored.
If Bridge runs in between, it rotates its Proton refresh token and the
snapshot becomes invalid. In practice, redoing the login is simpler.

### Rotating or revoking access

Rotating the signing key invalidates every outstanding token at once.
Rotate the passphrase too if it may have leaked.

```bash
aws ssm put-parameter --overwrite --name $PREFIX/oauth-signing-key --type SecureString --value "$(openssl rand -hex 32)"
aws ssm put-parameter --overwrite --name $PREFIX/oauth-passphrase --type SecureString --value '<new passphrase>'
```

The change is picked up within 30 minutes, or immediately with the
`send-command` from step 4. Clients re-prompt for the passphrase on next use.
The connector URL never changes.

### Looking at logs

```bash
aws ssm start-session --target <InstanceId>
sudo journalctl -u mcp -f                # MCP server
sudo journalctl -u protonmail-bridge -f  # Bridge
sudo journalctl -u caddy -f              # TLS / proxy
sudo cat /var/log/cloud-init-output.log  # first-boot provisioning
```

### Tearing it down

```bash
npx cdk destroy
aws ssm delete-parameters --names $PREFIX/username $PREFIX/password $PREFIX/oauth-passphrase $PREFIX/oauth-signing-key
```

`cdk destroy` removes the instance, its volume, the Elastic IP, the DNS
record, and the `image-uri` parameter. Pushed images stay in the CDK
bootstrap ECR repository; delete them there if you want them gone.

## How it works

- `bin/proton-mail-mcp.ts` reads `config.json` and instantiates the stack.
- `lib/mail-mcp-stack.ts` builds the Docker image from the repository root,
  publishes its URI to SSM, and creates the security group, IAM role, EC2
  instance, Elastic IP, Route 53 record, and the State Manager association.
- The instance's user data installs Docker, Caddy, the AWS CLI and the SSM
  agent, writes two systemd units (`protonmail-bridge`, `mcp`) and the script
  `/usr/local/bin/mcp-refresh-config`. That script fetches the SSM
  parameters, renders `/etc/proton-mcp/mcp.env` and `/etc/caddy/Caddyfile`,
  and pulls or restarts only what changed, so it is safe to run at any time.
- The instance role can pull from ECR and read parameters under `ssmPrefix`,
  and nothing else.
- `cdk.context.json` caches the VPC and hosted-zone lookups. It is gitignored
  because it contains your account-specific ids; CDK regenerates it with AWS
  credentials when it is missing.
