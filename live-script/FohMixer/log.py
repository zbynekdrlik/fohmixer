"""The ``fohmixer.<INSTANCE>`` logger: WARNING level, rotating file, never per message.

The file is ``logs/fohmixer-<INSTANCE>.log`` next to the script (1 MB x 3), or
in ``Config.LOG_DIR`` / ``$FOHMIXER_LOG_DIR``. It does not propagate to Live's
Log.txt, unless the log folder cannot be created (then Log.txt is the fallback).
"""

import logging
import os
from logging.handlers import RotatingFileHandler

from . import Config

MAX_BYTES = 1_000_000
BACKUP_COUNT = 3
_FORMAT = "%(asctime)s %(levelname)s %(name)s: %(message)s"
_MARK = "_fohmixer_handler"


def get_logger():
    return logging.getLogger(f"fohmixer.{Config.INSTANCE}")


def log_dir():
    if Config.LOG_DIR:
        return Config.LOG_DIR
    env = os.environ.get("FOHMIXER_LOG_DIR")
    if env:
        return env
    return os.path.join(os.path.dirname(os.path.abspath(__file__)), "logs")


def setup():
    """Attach the rotating file handler once per path (the logging registry is process-wide)."""
    logger = get_logger()
    logger.setLevel(logging.WARNING)
    directory = log_dir()
    path = os.path.join(directory, f"fohmixer-{Config.INSTANCE}.log")
    for handler in list(logger.handlers):
        if getattr(handler, _MARK, None) == path:
            return logger
        if getattr(handler, _MARK, None) is not None:
            logger.removeHandler(handler)
            handler.close()
    try:
        os.makedirs(directory, exist_ok=True)
        handler = RotatingFileHandler(
            path,
            maxBytes=MAX_BYTES,
            backupCount=BACKUP_COUNT,
            encoding="utf-8",
            delay=True,
        )
    except OSError as e:
        logger.propagate = True
        logger.warning("cannot write logs to %s (%s); logging to Live's Log.txt", directory, e)
        return logger
    handler.setFormatter(logging.Formatter(_FORMAT))
    setattr(handler, _MARK, path)
    logger.addHandler(handler)
    logger.propagate = False
    return logger
