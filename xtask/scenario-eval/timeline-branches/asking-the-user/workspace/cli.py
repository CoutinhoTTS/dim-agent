"""Command line entry for the batch tool."""
import sys


def main(argv):
    print("items:", " ".join(argv[1:]))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
