import { Kafka } from 'kafkajs';
import pino from 'pino';
import fs from 'node:fs';
import path from 'node:path';
import postgres from 'postgres';

const logger = pino({
  level: process.env.LOG_LEVEL || 'info',
  transport: process.env.NODE_ENV !== 'production' ? {
    target: 'pino-pretty',
    options: {
      colorize: true,
    },
  } : undefined,
});

function readCertFile(fileName: string): string | undefined {
  const filePath = path.join(import.meta.dir, fileName);
  if (fs.existsSync(filePath)) {
    return fs.readFileSync(filePath, 'utf8').trim();
  }
  return undefined;
}

function sanitizeCert(pem?: string): string | undefined {
  if (!pem) return undefined;
  let value = pem.trim().replace(/\\n/g, '\n');
  if (value.endsWith('----')) {
    value += '-';
  }
  return value;
}

const caCert = readCertFile('ca.pem') || sanitizeCert(process.env.KAFKA_CA_CERT);
const userCert = readCertFile('service.cert') || sanitizeCert(process.env.KAFKA_CERT);
const userKey = readCertFile('service.key') || sanitizeCert(process.env.KAFKA_AC);

const kafka = new Kafka({
  clientId: 'hami-consumer',
  brokers: [process.env.KAFKA_BROKER_URL || 'localhost:9092'],
  ssl: (caCert || userCert || userKey) ? {
    rejectUnauthorized: false,
    ca: caCert ? [caCert] : undefined,
    key: userKey,
    cert: userCert,
  } : undefined,
});

const topic = process.env.KAFKA_TOPIC || 'mails';
const groupId = process.env.KAFKA_GROUP_ID || 'hami-group';

const consumer = kafka.consumer({ groupId });

const pgHost = process.env.DB_HOST || 'localhost';
const pgPort = parseInt(process.env.DB_PORT || '21992', 10);
const pgUser = process.env.DB_USER || 'octagon';
const pgPassword = process.env.DB_PASSWORD || '7abd4f68488bb50ad5d7d35227d38be6b8d1b5f49328866a4c2bf52b40234d66';
const pgDatabase = process.env.DB_NAME || 'octagon_extra';

const sql = postgres({
  host: pgHost,
  port: pgPort,
  user: pgUser,
  password: pgPassword,
  database: pgDatabase,
});

async function initDb() {
  let retries = 10;
  while (retries > 0) {
    try {
      await sql`
        CREATE TABLE IF NOT EXISTS paragon_mails (
          id SERIAL PRIMARY KEY,
          message_id TEXT,
          sender TEXT,
          recipient TEXT,
          subject TEXT,
          plain_body TEXT,
          html_body TEXT,
          raw_payload JSONB NOT NULL,
          created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
        )
      `;
      logger.info('Database initialized successfully');
      return;
    } catch (err) {
      retries--;
      logger.warn({ err, retries }, 'Database connection failed, retrying');
      if (retries === 0) {
        throw err;
      }
      await new Promise((resolve) => setTimeout(resolve, 3000));
    }
  }
}

async function run() {
  await initDb();

  await consumer.connect();
  logger.info({ topic, groupId }, 'Kafka consumer connected');

  await consumer.subscribe({ topic, fromBeginning: true });
  logger.info({ topic }, 'Subscribed to topic');

  await consumer.run({
    eachMessage: async ({ topic, partition, message }) => {
      const rawValue = message.value?.toString();
      logger.info({
        topic,
        partition,
        offset: message.offset,
      }, 'Message received');

      if (!rawValue) return;

      try {
        const payload = JSON.parse(rawValue);
        const headers = payload.headers || {};
        const envelope = payload.envelope || {};

        const messageId = headers.message_id || null;
        const sender = envelope.from || headers.from || null;
        const recipient = envelope.to || headers.to || null;
        const subject = headers.subject || null;
        const plainBody = payload.plain || null;
        const htmlBody = payload.html || null;

        await sql`
          INSERT INTO paragon_mails (
            message_id,
            sender,
            recipient,
            subject,
            plain_body,
            html_body,
            raw_payload
          ) VALUES (
            ${messageId},
            ${sender},
            ${recipient},
            ${subject},
            ${plainBody},
            ${htmlBody},
            ${sql.json(payload)}
          )
        `;
        logger.info({ messageId }, 'Mail saved to database');
      } catch (err) {
        logger.error(err, 'Failed to parse or save mail');
      }
    },
  });

  const port = parseInt(process.env.PORT || '3000', 10);
  Bun.serve({
    port,
    async fetch(req) {
      const url = new URL(req.url);
      if (url.pathname === '/api/mails/last' && req.method === 'GET') {
        const email = url.searchParams.get('email');
        const limitParam = parseInt(url.searchParams.get('limit') || '50', 10);
        const limit = Math.min(isNaN(limitParam) ? 50 : limitParam, 150);

        try {
          let rows;
          if (email) {
            rows = await sql`
              SELECT id, message_id, sender, recipient, subject, plain_body, html_body, raw_payload, created_at
              FROM paragon_mails
              WHERE sender = ${email} OR recipient = ${email}
              ORDER BY id DESC
              LIMIT ${limit}
            `;
          } else {
            rows = await sql`
              SELECT id, message_id, sender, recipient, subject, plain_body, html_body, raw_payload, created_at
              FROM paragon_mails
              ORDER BY id DESC
              LIMIT ${limit}
            `;
          }
          return Response.json(rows);
        } catch (err: any) {
          logger.error(err, 'Failed to fetch mails');
          return new Response(JSON.stringify({ error: err.message }), {
            status: 500,
            headers: { 'Content-Type': 'application/json' },
          });
        }
      }
      return new Response('Not Found', { status: 404 });
    },
  });
  logger.info({ port }, 'HTTP server started');
}

run().catch((err) => {
  logger.error(err, 'Kafka consumer error');
  process.exit(1);
});

const errorTypes = ['unhandledRejection', 'uncaughtException'];
const signalTypes = ['SIGINT', 'SIGTERM', 'SIGQUIT'];

errorTypes.forEach((type) => {
  process.on(type, async (err) => {
    try {
      logger.error(err, `External error: ${type}`);
      await consumer.disconnect();
      await sql.end();
      process.exit(0);
    } catch (_) {
      process.exit(1);
    }
  });
});

signalTypes.forEach((type) => {
  process.on(type as any, async () => {
    try {
      logger.info(`Termination signal: ${type}`);
      await consumer.disconnect();
      await sql.end();
      process.exit(0);
    } catch (_) {
      process.exit(1);
    }
  });
});
