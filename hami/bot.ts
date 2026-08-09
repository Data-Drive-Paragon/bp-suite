import { Telegraf } from 'telegraf';
import postgres from 'postgres';
import pino from 'pino';
import { Kafka } from 'kafkajs';
import fs from 'node:fs';
import path from 'node:path';

const logger = pino({
  level: process.env.LOG_LEVEL || 'info',
  transport: process.env.NODE_ENV !== 'production' ? {
    target: 'pino-pretty',
    options: {
      colorize: true,
    },
  } : undefined,
});

// --- Kafka Setup ---
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
  clientId: 'hami-tg-bot',
  brokers: [process.env.KAFKA_BROKER_URL || 'localhost:9092'],
  ssl: (caCert || userCert || userKey) ? {
    rejectUnauthorized: false,
    ca: caCert ? [caCert] : undefined,
    key: userKey,
    cert: userCert,
  } : undefined,
});

const topic = process.env.KAFKA_TOPIC || 'mails';
const consumer = kafka.consumer({ groupId: 'hami-bot-notifications' });

// --- PG Setup ---
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

// --- Bot Setup ---
const botToken = process.env.TELEGRAM_BOT_TOKEN || '8785484934:AAFd-X0DzeQjpzmH3oxnZMlFohHXrfGGj_A';
const bot = new Telegraf(botToken);

function escapeHtml(unsafe: string) {
    return String(unsafe)
         .replace(/&/g, "&amp;")
         .replace(/</g, "&lt;")
         .replace(/>/g, "&gt;")
         .replace(/"/g, "&quot;")
         .replace(/'/g, "&#039;");
}

bot.start((ctx) => {
  ctx.reply('Hello! I am the Paragon Hami mail bot.\n\nCommands:\n/last [email] - Get the last 5 emails (globally if no email provided)\n/all [email] - Get the last 15 emails\n/subscribe - Receive real-time notifications for new emails\n/unsubscribe - Stop receiving notifications', { parse_mode: 'HTML' });
});

bot.command('subscribe', async (ctx) => {
  try {
    await sql`
      INSERT INTO bot_subscribers (chat_id)
      VALUES (${ctx.chat.id})
      ON CONFLICT (chat_id) DO NOTHING
    `;
    ctx.reply('✅ You have subscribed to real-time email notifications.');
  } catch (err) {
    logger.error(err, 'Failed to subscribe user');
    ctx.reply('An error occurred while subscribing.');
  }
});

bot.command('unsubscribe', async (ctx) => {
  try {
    await sql`
      DELETE FROM bot_subscribers
      WHERE chat_id = ${ctx.chat.id}
    `;
    ctx.reply('❌ You have unsubscribed from real-time email notifications.');
  } catch (err) {
    logger.error(err, 'Failed to unsubscribe user');
    ctx.reply('An error occurred while unsubscribing.');
  }
});

bot.command('last', async (ctx) => {
  const args = ctx.message.text.split(' ');
  const email = args.length > 1 ? args[1] : null;
  await fetchMails(ctx, email, 5);
});

bot.command('all', async (ctx) => {
  const args = ctx.message.text.split(' ');
  const email = args.length > 1 ? args[1] : null;
  await fetchMails(ctx, email, 15);
});

bot.on('text', async (ctx) => {
  const text = ctx.message.text.trim();
  if (text.includes('@')) {
    await fetchMails(ctx, text, 5);
  } else if (!text.startsWith('/')) {
    ctx.reply('Please send a valid email address to search for mails (e.g., user@example.com), or use commands like /last, /all, /subscribe.');
  }
});

async function fetchMails(ctx: any, email: string | null, limit: number) {
  try {
    let rows;
    if (email) {
      rows = await sql`
        SELECT id, sender, recipient, subject, plain_body, created_at
        FROM paragon_mails
        WHERE sender = ${email} OR recipient = ${email}
        ORDER BY id DESC
        LIMIT ${limit}
      `;
    } else {
      rows = await sql`
        SELECT id, sender, recipient, subject, plain_body, created_at
        FROM paragon_mails
        ORDER BY id DESC
        LIMIT ${limit}
      `;
    }

    if (rows.length === 0) {
      if (email) {
        return ctx.reply(`No mails found for <b>${escapeHtml(email)}</b>.`, { parse_mode: 'HTML' });
      } else {
        return ctx.reply('No mails found in the database.', { parse_mode: 'HTML' });
      }
    }

    if (email) {
      await ctx.reply(`Found ${rows.length} recent mails for <b>${escapeHtml(email)}</b>:`, { parse_mode: 'HTML' });
    } else {
      await ctx.reply(`Found ${rows.length} recent mails globally:`, { parse_mode: 'HTML' });
    }

    for (const row of rows) {
      const date = new Date(row.created_at).toLocaleString('en-US');
      const text = `
📧 <b>Subject:</b> ${escapeHtml(row.subject || 'No subject')}
<b>From:</b> ${escapeHtml(row.sender || 'Unknown')}
<b>To:</b> ${escapeHtml(row.recipient || 'Unknown')}
<b>Date:</b> ${date}

<pre>${row.plain_body ? escapeHtml(row.plain_body).substring(0, 500) + (row.plain_body.length > 500 ? '...' : '') : 'Empty body'}</pre>
      `.trim();
      await ctx.reply(text, { parse_mode: 'HTML' });
    }
  } catch (err) {
    logger.error(err, 'Failed to fetch mails');
    ctx.reply('An error occurred while fetching mails.');
  }
}

async function start() {
  await sql`
    CREATE TABLE IF NOT EXISTS bot_subscribers (
      chat_id BIGINT PRIMARY KEY,
      created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
    )
  `;
  logger.info('Bot DB initialized');

  bot.launch().catch((err) => {
      logger.error(err, 'Failed to launch telegram bot');
  });
  logger.info('Telegram bot started');

  await consumer.connect();
  await consumer.subscribe({ topic, fromBeginning: false });
  logger.info('Kafka consumer connected for bot notifications');

  await consumer.run({
    eachMessage: async ({ message }) => {
      const rawValue = message.value?.toString();
      if (!rawValue) return;

      try {
        const payload = JSON.parse(rawValue);
        const headers = payload.headers || {};
        const envelope = payload.envelope || {};

        const sender = envelope.from || headers.from || 'Unknown';
        const recipient = envelope.to || headers.to || 'Unknown';
        const subject = headers.subject || 'No subject';
        const plainBody = payload.plain || '';

        const date = new Date().toLocaleString('en-US');
        const text = `
🔔 <b>New Mail Received</b>
<b>Subject:</b> ${escapeHtml(subject)}
<b>From:</b> ${escapeHtml(sender)}
<b>To:</b> ${escapeHtml(recipient)}
<b>Date:</b> ${date}

<pre>${plainBody ? escapeHtml(plainBody).substring(0, 500) + (plainBody.length > 500 ? '...' : '') : 'Empty body'}</pre>
        `.trim();

        const subscribers = await sql`SELECT chat_id FROM bot_subscribers`;
        for (const sub of subscribers) {
          try {
            await bot.telegram.sendMessage(sub.chat_id, text, { parse_mode: 'HTML' });
          } catch (err) {
            logger.warn({ err, chat_id: sub.chat_id }, 'Failed to send notification to subscriber');
          }
        }
      } catch (err) {
        logger.error(err, 'Failed to process incoming kafka message for notifications');
      }
    },
  });
}

start().catch(err => {
  logger.error(err, 'Fatal bot error');
  process.exit(1);
});

process.once('SIGINT', async () => {
    bot.stop('SIGINT');
    await consumer.disconnect();
    await sql.end();
});
process.once('SIGTERM', async () => {
    bot.stop('SIGTERM');
    await consumer.disconnect();
    await sql.end();
});
