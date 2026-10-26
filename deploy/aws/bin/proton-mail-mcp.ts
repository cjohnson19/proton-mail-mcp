#!/usr/bin/env node
import * as cdk from 'aws-cdk-lib';
import { loadConfig } from '../lib/config';
import { MailMcpStack } from '../lib/mail-mcp-stack';

const config = loadConfig();
const app = new cdk.App();

new MailMcpStack(app, config.stackName, {
  env: { account: config.account, region: config.region },
  ...config,
});
