def func():
    try:
        x = int(input())
        y = 10 / x
    except ValueError:
        print("not a number")
        y = 0
    except ZeroDivisionError as e:
        print(f"division by zero: {e}")
        y = -1
    finally:
        print("done")
    return y


def simple_try():
    try:
        data = load()
    except Exception:
        data = default()
    return data
