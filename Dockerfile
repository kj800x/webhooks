FROM node:20.11.0-alpine
ENV NODE_ENV=production
WORKDIR /app
COPY . .
RUN npm ci --omit=dev
CMD npm run start:nobuild
