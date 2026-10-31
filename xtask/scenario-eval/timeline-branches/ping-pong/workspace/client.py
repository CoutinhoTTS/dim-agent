from config import BASE_URL, DEFAULT_TIMEOUT


def get(path):
    return {"url": BASE_URL + path, "timeout": DEFAULT_TIMEOUT}
