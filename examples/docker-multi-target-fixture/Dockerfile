# Two images out of one Dockerfile, sharing a stage — the shape `paws docker
# --target a,b` exists for. `shared` stands in for the expensive stage a real
# repo has here (a compile, a package install); building both targets in one
# invocation must run it once, not once per image.
FROM alpine:3.20 AS shared
RUN echo "shared stage ran" && echo shared >/shared.txt

FROM shared AS alpha
RUN cp /shared.txt /alpha.txt
CMD ["cat", "/alpha.txt"]

FROM shared AS beta
RUN cp /shared.txt /beta.txt
CMD ["cat", "/beta.txt"]
