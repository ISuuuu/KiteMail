import { spawn } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { pathToFileURL } from "node:url";

const DEFAULT_HOST = "127.0.0.1";
const DEFAULT_MAX_RETRIES = 5;

/**
 * 解析多 worktree 开发脚本命令行参数与环境变量。
 * 支持 `--demo` / `KITEMAIL_DEMO=1`、`--port <n>`、`--max-retries <n>` 以及 `--` 后透传给应用二进制的参数。
 */
export function parseDevTauriArgs(argv, env = process.env) {
  let demo = env.KITEMAIL_DEMO === "1" || env.KITEMAIL_DEMO === "true";
  let preferredPort = null;
  let maxRetries = DEFAULT_MAX_RETRIES;
  const extraTauriArgs = [];
  const extraAppArgs = [];

  if (env.KITEMAIL_DEV_PORT) {
    const parsed = Number(env.KITEMAIL_DEV_PORT);
    if (Number.isInteger(parsed) && parsed > 0 && parsed < 65536) {
      preferredPort = parsed;
    }
  }

  let passthroughApp = false;
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--") {
      passthroughApp = true;
      continue;
    }
    if (passthroughApp) {
      if (arg === "--demo") demo = true;
      extraAppArgs.push(arg);
      continue;
    }
    if (arg === "--demo") {
      demo = true;
      continue;
    }
    if (arg.startsWith("--port=")) {
      const parsed = Number(arg.slice("--port=".length));
      if (Number.isInteger(parsed) && parsed > 0 && parsed < 65536) {
        preferredPort = parsed;
      }
      continue;
    }
    if (arg === "--port" && i + 1 < argv.length) {
      const parsed = Number(argv[++i]);
      if (Number.isInteger(parsed) && parsed > 0 && parsed < 65536) {
        preferredPort = parsed;
      }
      continue;
    }
    if (arg.startsWith("--max-retries=")) {
      const parsed = Number(arg.slice("--max-retries=".length));
      if (Number.isInteger(parsed) && parsed > 0) {
        maxRetries = parsed;
      }
      continue;
    }
    if (arg === "--max-retries" && i + 1 < argv.length) {
      const parsed = Number(argv[++i]);
      if (Number.isInteger(parsed) && parsed > 0) {
        maxRetries = parsed;
      }
      continue;
    }
    extraTauriArgs.push(arg);
  }

  return {
    demo,
    preferredPort,
    maxRetries,
    extraTauriArgs,
    extraAppArgs,
  };
}

/** 构造覆盖 Tauri `build.devUrl` 的 JSON 配置字符串 */
export function buildTauriDevConfig(port, host = DEFAULT_HOST) {
  return JSON.stringify({
    build: {
      devUrl: `http://${host}:${port}`,
    },
  });
}

/** 构造 `pnpm tauri dev` 调用参数与环境变量补丁 */
export function buildTauriDevInvocation({
  port,
  demo = false,
  extraTauriArgs = [],
  extraAppArgs = [],
  host = DEFAULT_HOST,
}) {
  const args = [
    "tauri",
    "dev",
    "--config",
    buildTauriDevConfig(port, host),
    ...extraTauriArgs,
  ];
  const appArgs = [...extraAppArgs];
  if (demo && !appArgs.includes("--demo")) {
    appArgs.unshift("--demo");
  }
  if (appArgs.length > 0) {
    // Tauri CLI 约定（tauri dev --help）：第一个 `--` 后是 runner（cargo）参数，
    // 第二个 `--` 后才是应用参数。单 `--` 会把应用参数误传给 cargo 导致构建失败。
    args.push("--", "--", ...appArgs);
  }
  const envPatch = {
    KITEMAIL_DEV_PORT: String(port),
  };
  if (demo) {
    envPatch.KITEMAIL_DEMO = "1";
  }
  return {
    command: "pnpm",
    args,
    envPatch,
  };
}

/** 检测输出文本是否属于端口被占用的冲突错误 */
export function isPortConflictOutput(text) {
  if (!text) return false;
  return /EADDRINUSE|address already in use|Port \d+ is already in use|is already in use|os error 10048/i.test(
    text
  );
}

function tryBindPort(host, port) {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.unref();
    server.once("error", (err) => {
      reject(err);
    });
    server.listen({ host, port, exclusive: true }, () => {
      const address = server.address();
      const boundPort = typeof address === "object" && address ? address.port : 0;
      server.close((closeErr) => {
        if (closeErr) {
          reject(closeErr);
        } else if (boundPort > 0) {
          resolve(boundPort);
        } else {
          reject(new Error("无法获取已绑定的回环端口"));
        }
      });
    });
  });
}

/**
 * 探测 `127.0.0.1` 上的可用回环端口。
 * 若指定了 `preferredPort` 且未被占用或排除，优先尝试该端口，否则向 OS 申请临时空闲端口。
 */
export async function findFreeLoopbackPort({
  host = DEFAULT_HOST,
  preferredPort = null,
  excludePorts = new Set(),
} = {}) {
  if (
    Number.isInteger(preferredPort) &&
    preferredPort > 0 &&
    preferredPort < 65536 &&
    !excludePorts.has(preferredPort)
  ) {
    try {
      return await tryBindPort(host, preferredPort);
    } catch {
      // 首选端口已被占用时回退到 OS 分配的空闲回环端口
    }
  }

  for (let attempt = 0; attempt < 10; attempt++) {
    const candidate = await tryBindPort(host, 0);
    if (!excludePorts.has(candidate)) {
      return candidate;
    }
  }
  throw new Error("未能探测到可用的回环端口");
}

function quoteWinCmdArg(arg) {
  if (!/[\s"&|<>^()]/.test(arg)) return arg;
  return `"${arg.replace(/"/g, '\\"')}"`;
}

function spawnTauriAttempt(invocation, env) {
  const rootDir = path.resolve(import.meta.dirname, "..");
  const localTauriBin = path.join(rootDir, "node_modules", "@tauri-apps", "cli", "tauri.js");
  const childEnv = { ...env, ...invocation.envPatch };

  let child;
  if (fs.existsSync(localTauriBin)) {
    // 直接通过 Node 调度本地 @tauri-apps/cli，避免 Windows cmd.exe 吞掉 --config JSON 引号
    const tauriSubArgs = invocation.args.slice(1);
    child = spawn(process.execPath, [localTauriBin, ...tauriSubArgs], {
      cwd: rootDir,
      env: childEnv,
      stdio: ["inherit", "pipe", "pipe"],
    });
  } else if (process.platform === "win32") {
    const cmdLine = ["pnpm", ...invocation.args.map(quoteWinCmdArg)].join(" ");
    child = spawn(cmdLine, {
      cwd: rootDir,
      env: childEnv,
      shell: true,
      stdio: ["inherit", "pipe", "pipe"],
    });
  } else {
    child = spawn("pnpm", invocation.args, {
      cwd: rootDir,
      env: childEnv,
      stdio: ["inherit", "pipe", "pipe"],
    });
  }

  return new Promise((resolve) => {
    let outputTail = "";
    const appendTail = (chunk) => {
      outputTail = (outputTail + chunk.toString("utf8")).slice(-8192);
    };

    child.stdout?.on("data", (chunk) => {
      process.stdout.write(chunk);
      appendTail(chunk);
    });
    child.stderr?.on("data", (chunk) => {
      process.stderr.write(chunk);
      appendTail(chunk);
    });

    const onSigInt = () => child.kill("SIGINT");
    const onSigTerm = () => child.kill("SIGTERM");
    process.once("SIGINT", onSigInt);
    process.once("SIGTERM", onSigTerm);

    child.once("exit", (code, signal) => {
      process.off("SIGINT", onSigInt);
      process.off("SIGTERM", onSigTerm);
      resolve({
        code: code ?? (signal ? 1 : 0),
        signal,
        portConflict: (code ?? 0) !== 0 && isPortConflictOutput(outputTail),
      });
    });
  });
}

export async function runDevTauri(argv = process.argv.slice(2), env = process.env) {
  const parsed = parseDevTauriArgs(argv, env);
  const usedPorts = new Set();

  for (let attempt = 1; attempt <= parsed.maxRetries; attempt++) {
    const port = await findFreeLoopbackPort({
      host: DEFAULT_HOST,
      preferredPort: attempt === 1 ? parsed.preferredPort : null,
      excludePorts: usedPorts,
    });
    usedPorts.add(port);

    const invocation = buildTauriDevInvocation({
      port,
      demo: parsed.demo,
      extraTauriArgs: parsed.extraTauriArgs,
      extraAppArgs: parsed.extraAppArgs,
      host: DEFAULT_HOST,
    });

    console.info(
      `[dev:wt] 第 ${attempt}/${parsed.maxRetries} 次启动：KITEMAIL_DEV_PORT=${port}` +
        (parsed.demo ? " (demo)" : "")
    );

    const result = await spawnTauriAttempt(invocation, env);
    if (!result.portConflict || attempt === parsed.maxRetries) {
      return result.code;
    }

    console.warn(`[dev:wt] 端口 ${port} 发生冲突，正在切换空闲端口重试…`);
  }

  return 1;
}

const isMain =
  Boolean(process.argv[1]) &&
  !process.env.NODE_TEST_CONTEXT &&
  path.basename(process.argv[1]) === "dev-tauri.mjs" &&
  import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href;

if (isMain) {
  runDevTauri()
    .then((code) => process.exit(code))
    .catch((err) => {
      console.error("[dev:wt] 启动失败:", err);
      process.exit(1);
    });
}
