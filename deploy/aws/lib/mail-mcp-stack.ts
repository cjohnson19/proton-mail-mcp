import * as cdk from 'aws-cdk-lib';
import { Construct } from 'constructs';
import * as ec2 from 'aws-cdk-lib/aws-ec2';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as route53 from 'aws-cdk-lib/aws-route53';
import * as ssm from 'aws-cdk-lib/aws-ssm';
import { DockerImageAsset, Platform } from 'aws-cdk-lib/aws-ecr-assets';
import * as fs from 'node:fs';
import * as path from 'node:path';
import type { DeployConfig } from './config';

type MailMcpStackProps = cdk.StackProps & Omit<DeployConfig, 'account' | 'region' | 'stackName'>;

const DEBIAN_12_AMI_PARAMETER = '/aws/service/debian/release/12/latest/amd64';

export class MailMcpStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props: MailMcpStackProps) {
    super(scope, id, props);

    const appDir = path.resolve(__dirname, '..', '..', '..');
    if (!fs.existsSync(path.join(appDir, 'Dockerfile'))) {
      throw new Error(`Dockerfile not found at ${appDir}; this stack must live at deploy/aws inside the proton-mail-mcp checkout`);
    }
    const image = new DockerImageAsset(this, 'AppImage', {
      directory: appDir,
      platform: Platform.LINUX_AMD64,
    });

    const vpc = ec2.Vpc.fromLookup(this, 'DefaultVpc', { isDefault: true });

    const sg = new ec2.SecurityGroup(this, 'Sg', {
      vpc,
      allowAllOutbound: true,
      description: 'proton-mail-mcp inbound (80/443)',
    });
    [ec2.Peer.anyIpv4(), ec2.Peer.anyIpv6()].forEach((peer) => {
      sg.addIngressRule(peer, ec2.Port.tcp(80));
      sg.addIngressRule(peer, ec2.Port.tcp(443));
    });

    const role = new iam.Role(this, 'Role', {
      assumedBy: new iam.ServicePrincipal('ec2.amazonaws.com'),
      managedPolicies: [
        iam.ManagedPolicy.fromAwsManagedPolicyName('AmazonEC2ContainerRegistryReadOnly'),
        iam.ManagedPolicy.fromAwsManagedPolicyName('AmazonSSMManagedInstanceCore'),
      ],
    });
    image.repository.grantPull(role);
    role.addToPolicy(
      new iam.PolicyStatement({
        actions: ['ssm:GetParameter'],
        resources: [`arn:aws:ssm:${this.region}:${this.account}:parameter${props.ssmPrefix}/*`],
      }),
    );

    const ecrHost = `${this.account}.dkr.ecr.${this.region}.amazonaws.com`;

    // The app image URI lives in SSM, not in user data: baking it into the
    // instance would replace the instance (and lose Bridge login state) on
    // every app deploy. cdk deploy updates the parameter; the refresh script
    // below rolls the new image out in place.
    new ssm.StringParameter(this, 'ImageUriParam', {
      parameterName: `${props.ssmPrefix}/image-uri`,
      stringValue: image.imageUri,
    });

    const redirectAllowlistLine =
      props.oauthRedirectAllowlist.length > 0
        ? `MCP_OAUTH_REDIRECT_ALLOWLIST=${props.oauthRedirectAllowlist.join(',')}\n`
        : '';

    // Fetches secrets + the current image URI from SSM, renders the MCP env
    // files and the Caddyfile, then pulls/restarts only what changed.
    // Idempotent, so State Manager runs it on a schedule. Everything in user
    // data must stay ASCII: cloud-init chokes on anything else.
    const refreshScript = `#!/bin/bash
set -eu
get() { aws ssm get-parameter --region ${this.region} --name "${props.ssmPrefix}/\$1" --with-decryption --query Parameter.Value --output text; }
USERNAME=\$(get username)
PASSWORD=\$(get password)
OAUTH_PASSPHRASE=\$(get oauth-passphrase)
OAUTH_SIGNING_KEY=\$(get oauth-signing-key)
IMAGE=\$(get image-uri)
install -d -m 700 /etc/proton-mcp
umask 077
render() { # render() TARGET <<EOF... writes only on change (preserving existing perms), echoes 1 if changed
  tmp=\$(mktemp)
  cat > "\$tmp"
  if cmp -s "\$tmp" "\$1" 2>/dev/null; then rm -f "\$tmp"; echo 0; else cat "\$tmp" > "\$1"; rm -f "\$tmp"; echo 1; fi
}
mcp_changed=\$(render /etc/proton-mcp/mcp.env <<EOF
PROTON_HOSTNAME=protonmail-bridge
PROTON_PORT=143
PROTON_USERNAME=\$USERNAME
PROTON_PASSWORD=\$PASSWORD
MCP_BIND=0.0.0.0:8080
MCP_PUBLIC_URL=https://${props.domainName}
MCP_OAUTH_PASSPHRASE=\$OAUTH_PASSPHRASE
MCP_OAUTH_SIGNING_KEY=\$OAUTH_SIGNING_KEY
${redirectAllowlistLine}EOF
)
image_changed=\$(render /etc/proton-mcp/image.env <<EOF
MCP_IMAGE=\$IMAGE
EOF
)
caddy_changed=\$(render /etc/caddy/Caddyfile <<EOF
${props.domainName} {
    reverse_proxy localhost:8080
}
EOF
)
if [ "\$image_changed" = 1 ]; then
  aws ecr get-login-password --region ${this.region} | docker login --username AWS --password-stdin ${ecrHost}
  docker pull "\$IMAGE"
fi
if [ "\$mcp_changed" = 1 ] || [ "\$image_changed" = 1 ]; then
  systemctl try-restart mcp.service || true
fi
if [ "\$caddy_changed" = 1 ]; then
  systemctl reload caddy || systemctl restart caddy
fi
echo "refresh done (mcp=\$mcp_changed image=\$image_changed caddy=\$caddy_changed)"
`;

    const bridgeUnit = [
      '[Unit]',
      'Description=Proton Mail Bridge container',
      'After=docker.service network-online.target',
      'Wants=network-online.target',
      'Requires=docker.service',
      '',
      '[Service]',
      'Restart=always',
      'RestartSec=5',
      'ExecStartPre=-/usr/bin/docker rm -f protonmail-bridge',
      'ExecStartPre=-/usr/bin/docker network create mcp-net',
      `ExecStart=/usr/bin/docker run --rm --name protonmail-bridge --network mcp-net -v /opt/bridge-data:/root -e PROTON_BRIDGE_HOST=127.0.0.1 -e PROTON_BRIDGE_SMTP_PORT=1025 -e PROTON_BRIDGE_IMAP_PORT=1143 -e CONTAINER_SMTP_PORT=25 -e CONTAINER_IMAP_PORT=143 ${props.bridgeImage}`,
      'ExecStop=/usr/bin/docker stop protonmail-bridge',
      '',
      '[Install]',
      'WantedBy=multi-user.target',
      '',
    ].join('\n');

    const mcpUnit = [
      '[Unit]',
      'Description=Proton Mail MCP server container',
      'After=docker.service protonmail-bridge.service',
      'Wants=protonmail-bridge.service',
      'Requires=docker.service',
      '',
      '[Service]',
      'Restart=always',
      'RestartSec=5',
      'EnvironmentFile=/etc/proton-mcp/image.env',
      'ExecStartPre=-/usr/bin/docker rm -f mcp',
      'ExecStart=/usr/bin/docker run --rm --name mcp --network mcp-net --env-file /etc/proton-mcp/mcp.env -p 127.0.0.1:8080:8080 ${MCP_IMAGE}',
      'ExecStop=/usr/bin/docker stop mcp',
      '',
      '[Install]',
      'WantedBy=multi-user.target',
      '',
    ].join('\n');

    const userData = ec2.UserData.forLinux();
    userData.addCommands(
      'set -eux',
      'apt-get update',
      'DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends ca-certificates curl wget docker.io awscli caddy',
      'wget -q -O /tmp/ssm.deb https://s3.amazonaws.com/ec2-downloads-windows/SSMAgent/latest/debian_amd64/amazon-ssm-agent.deb',
      'dpkg -i /tmp/ssm.deb',
      'systemctl enable --now amazon-ssm-agent',
      'systemctl enable --now docker',
      `cat > /usr/local/bin/mcp-refresh-config <<'REFRESHEOF'\n${refreshScript}REFRESHEOF`,
      'chmod +x /usr/local/bin/mcp-refresh-config',
      `cat > /etc/systemd/system/protonmail-bridge.service <<'BRIDGEEOF'\n${bridgeUnit}BRIDGEEOF`,
      `cat > /etc/systemd/system/mcp.service <<'MCPEOF'\n${mcpUnit}MCPEOF`,
      'systemctl daemon-reload',
      'systemctl enable --now protonmail-bridge.service',
      '/usr/local/bin/mcp-refresh-config || echo "WARN: set SSM parameters then run mcp-refresh-config"',
      'systemctl enable --now mcp.service',
    );

    // Without a pinned amiId the "latest" Debian 12 image is resolved on
    // every deploy. When it moves, CloudFormation replaces the instance and
    // the Bridge login in /opt/bridge-data is lost. Pin amiId in config.json
    // (the AmiId output tells you what is running) once the first deploy is up.
    const machineImage = props.amiId
      ? ec2.MachineImage.genericLinux({ [this.region]: props.amiId })
      : ec2.MachineImage.fromSsmParameter(DEBIAN_12_AMI_PARAMETER, { os: ec2.OperatingSystemType.LINUX });

    const instance = new ec2.Instance(this, 'Instance', {
      vpc,
      vpcSubnets: { subnetType: ec2.SubnetType.PUBLIC },
      instanceType: new ec2.InstanceType(props.instanceType),
      machineImage,
      securityGroup: sg,
      role,
      userData,
      userDataCausesReplacement: true,
      blockDevices: [
        {
          deviceName: '/dev/xvda',
          volume: ec2.BlockDeviceVolume.ebs(16, {
            volumeType: ec2.EbsDeviceVolumeType.GP3,
            encrypted: true,
            deleteOnTermination: true,
          }),
        },
      ],
    });

    // Rolls out SSM parameter changes (new app image, rotated secrets)
    // without a deploy touching the instance. mcp-refresh-config is
    // change-detecting, so the periodic runs are no-ops between deploys.
    // 30 minutes is State Manager's minimum rate interval.
    new ssm.CfnAssociation(this, 'RefreshAssociation', {
      name: 'AWS-RunShellScript',
      targets: [{ key: 'InstanceIds', values: [instance.instanceId] }],
      scheduleExpression: 'rate(30 minutes)',
      parameters: { commands: ['/usr/local/bin/mcp-refresh-config'] },
    });

    const eip = new ec2.CfnEIP(this, 'Eip', {
      instanceId: instance.instanceId,
      domain: 'vpc',
    });

    const zone = route53.HostedZone.fromLookup(this, 'Zone', { domainName: props.hostedZoneName });

    new route53.ARecord(this, 'ARecord', {
      zone,
      recordName: props.domainName,
      target: route53.RecordTarget.fromIpAddresses(eip.ref),
      ttl: cdk.Duration.minutes(5),
    });

    new cdk.CfnOutput(this, 'ConnectorUrl', { value: `https://${props.domainName}/mcp` });
    new cdk.CfnOutput(this, 'PublicIp', { value: eip.ref });
    new cdk.CfnOutput(this, 'InstanceId', { value: instance.instanceId });
    new cdk.CfnOutput(this, 'AmiId', { value: instance.instance.imageId ?? '' });
  }
}
