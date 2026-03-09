def my_func():
    total = 0
    for i in range(10):
        if i % 2 == 0:
            continue
        total += i
    return total


def while_loop():
    x = 100
    while x > 1:
        x = x // 2
    return x


def break_loop():
    for i in range(100):
        if i > 10:
            break
        print(i)
    return i
