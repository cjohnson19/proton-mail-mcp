import * as fs from 'node:fs';
import * as path from 'node:path';

export interface DeployConfig {
  account: string;
  region: string;
  domainName: string;
  hostedZoneName: string;
  instanceType: string;
  amiId?: string;
  bridgeImage: string;
  ssmPrefix: string;
  stackName: string;
  oauthRedirectAllowlist: string[];
}

const defaults = {
  instanceType: 't3a.micro',
  bridgeImage: 'ghcr.io/videocurio/proton-mail-bridge:v3.24.2',
  ssmPrefix: '/proton-mail-mcp',
  stackName: 'MailMcpStack',
  oauthRedirectAllowlist: [] as string[],
};

const required = ['account', 'region', 'domainName', 'hostedZoneName'] as const;

export function loadConfig(file = path.join(__dirname, '..', 'config.json')): DeployConfig {
  if (!fs.existsSync(file)) {
    throw new Error(`${file} not found. Copy config.example.json to config.json and fill it in.`);
  }
  const raw = JSON.parse(fs.readFileSync(file, 'utf8')) as Record<string, unknown>;

  const missing = required.filter((key) => typeof raw[key] !== 'string' || raw[key] === '');
  if (missing.length > 0) {
    throw new Error(`config.json is missing required fields: ${missing.join(', ')}`);
  }
  if (!/^\d{12}$/.test(raw.account as string)) {
    throw new Error('config.json: "account" must be a 12-digit AWS account id');
  }
  if (!(raw.domainName as string).endsWith(raw.hostedZoneName as string)) {
    throw new Error('config.json: "domainName" must be inside "hostedZoneName"');
  }
  if (typeof raw.ssmPrefix === 'string' && !raw.ssmPrefix.startsWith('/')) {
    throw new Error('config.json: "ssmPrefix" must start with "/"');
  }

  const config = { ...defaults, ...raw } as DeployConfig & { amiId?: string | null };
  return { ...config, amiId: config.amiId ?? undefined };
}
