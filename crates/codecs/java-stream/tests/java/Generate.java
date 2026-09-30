// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Writes the Java object serialization streams the `axioval-java-stream`
// round-trip tests read. Every class here is a test class of this program:
// the streams exercise the stream protocol, not any particular application.
//
// Regenerate (JDK 17 or newer), from the crate directory:
//
//   javac -d ../../../target/java-stream tests/java/Generate.java
//   java -cp ../../../target/java-stream Generate tests/data
//
// Output is deterministic for a given JDK and source: every test class
// declares its serialVersionUID. Proxy class names depend on the JDK, and the
// stack trace in exception.ser on this file's line numbers.

import java.io.ByteArrayOutputStream;
import java.io.Externalizable;
import java.io.IOException;
import java.io.ObjectInput;
import java.io.ObjectInputStream;
import java.io.ObjectOutput;
import java.io.ObjectOutputStream;
import java.io.ObjectStreamClass;
import java.io.ObjectStreamConstants;
import java.io.OutputStream;
import java.io.Serializable;
import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Method;
import java.lang.reflect.Proxy;
import java.math.BigDecimal;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.EnumMap;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.LinkedList;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;

// Test classes hold deliberately non-serializable types; the warning is noise.
@SuppressWarnings("serial")
public final class Generate {
    private Generate() {}

    /** One field of every primitive type, at awkward values. */
    static final class Primitives implements Serializable {
        private static final long serialVersionUID = 1L;
        byte b = -12;
        char c = 'é';
        double d = Math.PI;
        // writeDouble writes doubleToLongBits, which canonicalizes NaN.
        double nan = Double.longBitsToDouble(0x7ff8_0000_dead_beefL);
        float f = -0.0f;
        int i = Integer.MIN_VALUE;
        long l = Long.MAX_VALUE;
        short s = Short.MIN_VALUE;
        boolean z = true;
        boolean off = false;
    }

    /** Strings in every modified UTF-8 shape. */
    static final class Strings implements Serializable {
        private static final long serialVersionUID = 2L;
        String empty = "";
        String ascii = "plain";
        String nul = "a\u0000b";
        String twoByte = "Grüße";
        String threeByte = "€世";
        String astral = "😀 𝄞";
        String loneSurrogate = "x\uD800y";
        String absent = null;
    }

    /** One array of every element type. */
    static final class ArrayHolder implements Serializable {
        private static final long serialVersionUID = 3L;
        byte[] bytes = {0, 1, -1, 127, -128};
        char[] chars = {'a', 'é', '￿'};
        double[] doubles = {0.0, -1.5, Double.POSITIVE_INFINITY};
        float[] floats = {1.25f, Float.MIN_VALUE};
        int[] ints = {1, -2, Integer.MAX_VALUE};
        long[] longs = {Long.MIN_VALUE, 0L};
        short[] shorts = {7, -7};
        boolean[] booleans = {true, false, true};
        String[] strings = {"one", null, "one"};
        int[][] matrix = {{1, 2}, {}, null};
        Object[] mixed = {Integer.valueOf(5), "s", new long[] {9L}, null};
        int[] empty = {};
    }

    /** A graph node: shared payloads, back-references and cycles. */
    static final class Node implements Serializable {
        private static final long serialVersionUID = 4L;
        String name;
        Node next;
        Object payload;

        Node(String name) {
            this.name = name;
        }
    }

    enum Colour {
        RED,
        GREEN,
        BLUE {
            @Override
            public String toString() {
                return "blue";
            }
        }
    }

    static final class Palette implements Serializable {
        private static final long serialVersionUID = 5L;
        Colour primary = Colour.RED;
        Colour secondary = Colour.BLUE;
        Colour again = Colour.RED;
        Colour[] all = Colour.values();
    }

    /** A class whose writeObject appends block data and objects. */
    static final class Custom implements Serializable {
        private static final long serialVersionUID = 6L;
        int count = 3;
        transient String cache = "not written";

        private void writeObject(ObjectOutputStream out) throws IOException {
            out.defaultWriteObject();
            out.writeInt(42);
            out.writeUTF("custom");
            out.writeObject(new int[] {1, 2, 3});
            byte[] large = new byte[3000];
            for (int k = 0; k < large.length; k++) {
                large[k] = (byte) k;
            }
            out.write(large);
            out.writeObject("after");
            out.writeDouble(2.5);
        }

        private void readObject(ObjectInputStream in) throws IOException, ClassNotFoundException {
            in.defaultReadObject();
        }
    }

    /** A class that writes only custom data after an empty default. */
    static final class OnlyCustom implements Serializable {
        private static final long serialVersionUID = 7L;

        private void writeObject(ObjectOutputStream out) throws IOException {
            out.defaultWriteObject();
        }
    }

    public static final class External implements Externalizable {
        private static final long serialVersionUID = 8L;
        int value = 11;
        String label = "ext";

        public External() {}

        @Override
        public void writeExternal(ObjectOutput out) throws IOException {
            out.writeInt(value);
            out.writeObject(label);
            out.writeObject(new Node("inside"));
            out.writeLong(-1L);
        }

        @Override
        public void readExternal(ObjectInput in) throws IOException, ClassNotFoundException {
            value = in.readInt();
            label = (String) in.readObject();
            in.readObject();
            in.readLong();
        }
    }

    public interface Greeter {
        String greet(String who);
    }

    static final class Handler implements InvocationHandler, Serializable {
        private static final long serialVersionUID = 9L;
        String prefix = "hello ";

        @Override
        public Object invoke(Object proxy, Method method, Object[] args) {
            return prefix + args[0];
        }
    }

    /** Not serializable: its state is not written, only its subclasses'. */
    static class Base {
        int notWritten = 7;

        Base() {}
    }

    static class Animal extends Base implements Serializable {
        private static final long serialVersionUID = 10L;
        String kind = "animal";
    }

    static class Mammal extends Animal {
        private static final long serialVersionUID = 11L;
        int legs = 4;

        private void writeObject(ObjectOutputStream out) throws IOException {
            out.defaultWriteObject();
            out.writeBoolean(true);
        }
    }

    static final class Dog extends Mammal {
        private static final long serialVersionUID = 12L;
        String name = "Rex";
        Animal friend;
    }

    /** Holds something that cannot be serialized, which aborts the write. */
    static final class Broken implements Serializable {
        private static final long serialVersionUID = 13L;
        String before = "written";
        Object thing = new Object();
    }

    /** Writes class annotations, as a stream carrying code locations would. */
    static final class AnnotatingStream extends ObjectOutputStream {
        AnnotatingStream(OutputStream out) throws IOException {
            super(out);
        }

        @Override
        protected void annotateClass(Class<?> cl) throws IOException {
            writeUTF("origin:" + cl.getSimpleName());
            writeObject(cl.getSimpleName().length() % 2 == 0 ? "even" : null);
        }

        @Override
        protected void annotateProxyClass(Class<?> cl) throws IOException {
            writeInt(cl.getInterfaces().length);
        }
    }

    interface Body {
        void write(ObjectOutputStream out) throws IOException;
    }

    static void emit(Path dir, String name, Body body) throws IOException {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        ObjectOutputStream out = new ObjectOutputStream(bytes);
        body.write(out);
        out.flush();
        Files.write(dir.resolve(name + ".ser"), bytes.toByteArray());
    }

    static Greeter proxy() {
        return (Greeter) Proxy.newProxyInstance(
                Generate.class.getClassLoader(), new Class<?>[] {Greeter.class}, new Handler());
    }

    public static void main(String[] args) throws Exception {
        Path dir = Path.of(args.length > 0 ? args[0] : ".");
        Files.createDirectories(dir);

        emit(dir, "primitives", out -> out.writeObject(new Primitives()));

        emit(dir, "strings", out -> {
            out.writeObject(new Strings());
            String shared = "shared";
            out.writeObject(shared);
            out.writeObject(shared);
            // More than 65535 bytes of modified UTF-8: TC_LONGSTRING.
            out.writeObject("é".repeat(40_000) + "😀");
        });

        emit(dir, "arrays", out -> {
            out.writeObject(new ArrayHolder());
            Object[] self = new Object[2];
            self[0] = self;
            self[1] = "tail";
            out.writeObject(self);
        });

        emit(dir, "graph", out -> {
            Node a = new Node("a");
            Node b = new Node("b");
            Node c = new Node("c");
            Object shared = new ArrayList<>(List.of("x", "y"));
            a.next = b;
            b.next = c;
            c.next = a;
            a.payload = shared;
            c.payload = shared;
            Node self = new Node("self");
            self.next = self;
            self.payload = self;
            out.writeObject(a);
            out.writeObject(self);
            out.writeObject(b);
            Node chain = null;
            for (int k = 0; k < 64; k++) {
                Node link = new Node("n" + k);
                link.next = chain;
                chain = link;
            }
            out.writeObject(chain);
        });

        emit(dir, "enums", out -> {
            out.writeObject(new Palette());
            out.writeObject(Colour.GREEN);
            out.writeObject(Colour.BLUE);
            EnumMap<Colour, String> map = new EnumMap<>(Colour.class);
            map.put(Colour.GREEN, "g");
            out.writeObject(map);
        });

        emit(dir, "custom", out -> {
            out.writeObject(new Custom());
            out.writeObject(new OnlyCustom());
        });

        emit(dir, "externalizable", out -> {
            out.writeObject(new External());
            out.writeObject(new External());
        });

        emit(dir, "proxy", out -> {
            Greeter g = proxy();
            out.writeObject(g);
            out.writeObject(proxy());
            out.writeObject(g);
        });

        emit(dir, "hierarchy", out -> {
            Dog dog = new Dog();
            Dog other = new Dog();
            other.name = "Fido";
            other.friend = dog;
            dog.friend = new Animal();
            out.writeObject(other);
            out.writeObject(new Mammal());
        });

        emit(dir, "collections", out -> {
            ArrayList<Object> list = new ArrayList<>(List.of("a", 1, 2L, 3.5));
            HashMap<String, Integer> hash = new HashMap<>();
            hash.put("one", 1);
            hash.put("two", 2);
            LinkedHashMap<String, Object> linked = new LinkedHashMap<>();
            linked.put("list", list);
            linked.put("hash", hash);
            TreeMap<String, String> tree = new TreeMap<>(Map.of("k", "v", "a", "b"));
            out.writeObject(linked);
            out.writeObject(tree);
            out.writeObject(new LinkedList<>(Arrays.asList("p", "q")));
            out.writeObject(new BigDecimal("12345678901234567890.0625"));
            out.writeObject(Arrays.asList(1, 2, 3));
        });

        {
            ByteArrayOutputStream bytes = new ByteArrayOutputStream();
            AnnotatingStream out = new AnnotatingStream(bytes);
            out.writeObject(new Dog());
            out.writeObject(proxy());
            out.writeObject(Colour.BLUE);
            out.flush();
            Files.write(dir.resolve("annotated.ser"), bytes.toByteArray());
        }

        emit(dir, "toplevel", out -> {
            out.writeInt(7);
            out.writeUTF("between");
            Node node = new Node("before reset");
            out.writeObject(node);
            out.writeObject(node);
            out.reset();
            out.writeObject(node);
            out.writeObject(String.class);
            out.writeObject(int[].class);
            out.writeObject(Colour.class);
            out.writeObject(Greeter.class);
            out.writeObject(ObjectStreamClass.lookup(Dog.class));
            out.writeUnshared(node);
            out.writeUnshared(node);
            out.writeObject(null);
            byte[] big = new byte[70_000];
            Arrays.fill(big, (byte) 0x5a);
            out.write(big);
            out.writeLong(99L);
        });

        emit(dir, "exception", out -> {
            out.writeObject("first");
            try {
                out.writeObject(new Broken());
                throw new IllegalStateException("expected the write to abort");
            } catch (java.io.NotSerializableException expected) {
                // The stream now carries TC_EXCEPTION and the exception object.
            }
            out.writeObject("after");
        });

        emit(dir, "protocol1", out -> {
            out.useProtocolVersion(ObjectStreamConstants.PROTOCOL_VERSION_1);
            out.writeObject(new Dog());
            out.writeObject(new Custom());
        });

        emit(dir, "protocol1-external", out -> {
            out.useProtocolVersion(ObjectStreamConstants.PROTOCOL_VERSION_1);
            out.writeObject(new External());
        });
    }
}
